/// Core proxy pipeline handler.
///
/// Implements the full request lifecycle:
/// 1. Route matching
/// 2. Authentication
/// 3. Template context construction
/// 4. Lookup pre-resolution (async, before sync template rendering)
/// 5. Pre-request hook execution
/// 6. Upstream proxying (streaming or buffered)
/// 7. Response forwarding + post-response usage logging
use crate::auth::{AuthError, AuthMethod, Authenticator, Identity, JwtMinter, SharedJwtMinter};
use crate::cache::GateCache;
use crate::config::{
    LookupRegistry, TemplateContext, TemplateEngine,
    lookup::collect_hook_templates,
    types::{GateConfig, PreRequestHook},
};
use crate::db::{Database, UsageEvent};
use crate::proxy::{CompiledRoute, Router as GateRouter, SharedRouter};
use crate::stream::SseStreamProcessor;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::StreamExt;
use http::request::Parts;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Headers that must not be forwarded to the upstream (hop-by-hop).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Application state shared across all requests.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<GateConfig>>,
    pub router: SharedRouter,
    pub auth_providers: Arc<HashMap<String, Arc<dyn Authenticator>>>,
    pub jwt_minter: SharedJwtMinter,
    pub cache: Arc<GateCache>,
    pub db: Option<Arc<Database>>,
    pub http_client: reqwest::Client,
    pub lookup_registry: Arc<LookupRegistry>,
}

/// The main proxy handler — catches all requests on the proxy port.
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let span = tracing::info_span!(
        "proxy_request",
        request_id = %request_id,
        method = %req.method(),
        uri = %req.uri(),
    );
    let _enter = span.enter();

    match handle_request(state, req, &request_id).await {
        Ok(response) => response,
        Err(status) => {
            warn!(request_id = %request_id, status = %status, "request failed");
            status.into_response()
        }
    }
}

async fn handle_request(
    state: Arc<AppState>,
    req: axum::extract::Request,
    request_id: &str,
) -> Result<Response, StatusCode> {
    // ── 1. Extract request parts ───────────────────────────────────────────
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    // Determine host from Host header
    let host = headers
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    let path = uri.path();
    let method_str = method.as_str();

    // ── 2. Route matching ──────────────────────────────────────────────────
    let router = state.router.read().await;
    let matched_route = match router.match_route(&host, path, method_str) {
        Some(r) => r.clone(),
        None => {
            info!(host = %host, path = %path, "no route matched");
            return Err(StatusCode::NOT_FOUND);
        }
    };
    drop(router);

    let route_id = matched_route.config.id.clone();
    info!(
        request_id = %request_id,
        route_id = %route_id,
        "route matched"
    );

    // ── 3. Authentication ──────────────────────────────────────────────────
    let auth_provider_name = matched_route
        .config
        .auth
        .as_deref()
        .or(matched_route.site.default_auth.as_deref());

    let auth_result = if let Some(provider_name) = auth_provider_name {
        match state.auth_providers.get(provider_name) {
            Some(auth) => match auth.authenticate(&parts).await {
                Ok(result) => result,
                Err(AuthError::Unauthorized(msg)) => {
                    warn!(request_id = %request_id, provider = %provider_name, reason = %msg, "authentication failed");
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Err(AuthError::ProviderError(msg)) => {
                    error!(request_id = %request_id, provider = %provider_name, error = %msg, "auth provider error");
                    return Err(StatusCode::BAD_GATEWAY);
                }
                Err(AuthError::NotConfigured) => {
                    crate::auth::AuthResult {
                        identity: Identity::anonymous("anonymous"),
                        method: AuthMethod::Anonymous,
                    }
                }
            },
            None => {
                error!(request_id = %request_id, provider = %provider_name, "configured auth provider not found");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    } else {
        crate::auth::AuthResult {
            identity: Identity::anonymous("anonymous"),
            method: AuthMethod::Anonymous,
        }
    };

    let identity = auth_result.identity;
    info!(
        request_id = %request_id,
        user_id = %identity.id,
        "authenticated"
    );

    // ── 4. Read request body ───────────────────────────────────────────────
    const MAX_BODY_SIZE: usize = 32 * 1024 * 1024; // 32 MiB
    let body_bytes = axum::body::to_bytes(body, MAX_BODY_SIZE)
        .await
        .map_err(|e| {
            warn!(request_id = %request_id, error = %e, "failed to read request body");
            StatusCode::BAD_REQUEST
        })?;

    // ── 5. Build template context ──────────────────────────────────────────
    let body_value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
    };

    let mut api_key_ctx = HashMap::new();
    if let AuthMethod::ApiKey { client_id, scopes } = &auth_result.method {
        api_key_ctx.insert("client_id".to_string(), client_id.clone());
        api_key_ctx.insert("scopes".to_string(), scopes.join(","));
    }

    let mut template_ctx = TemplateContext::new(
        identity.to_value(),
        body_value,
        request_id.to_string(),
        api_key_ctx,
    );

    // ── 5b. Pre-resolve async lookups ─────────────────────────────────────
    {
        let hook_templates = collect_hook_templates(&matched_route.config.hooks.pre_request);
        let resolved = state.lookup_registry.resolve_all(&hook_templates, &template_ctx).await;
        template_ctx.lookups = resolved;
    }

    // ── 6. Pre-request hooks ───────────────────────────────────────────────
    let mut injected_headers: HashMap<String, String> = HashMap::new();
    let mut body_overrides: HashMap<String, String> = HashMap::new();
    let mut minted_jwt: Option<String> = None;

    for hook in &matched_route.config.hooks.pre_request {
        match hook {
            PreRequestHook::ClaimsEnhancement { config } => {
                // Inject headers via template
                for (header_name, template) in &config.inject_headers {
                    let value = TemplateEngine::render(template, &template_ctx);
                    injected_headers.insert(header_name.clone(), value);
                }
                // Optionally mint a JWT
                if let Some(mint_cfg) = &config.mint_jwt {
                    if mint_cfg.enabled {
                        let minter_guard = state.jwt_minter.read().await;
                        if let Some(minter) = minter_guard.as_ref() {
                            match minter.mint(&identity, Some(&mint_cfg.additional_claims), None) {
                                Ok(token) => {
                                    minted_jwt = Some(token);
                                }
                                Err(e) => {
                                    warn!(request_id = %request_id, error = %e, "JWT minting failed");
                                }
                            }
                        }
                    }
                }
            }
            PreRequestHook::BodyTransform { config } => {
                for (field, template) in &config.set_fields {
                    let value = TemplateEngine::render(template, &template_ctx);
                    body_overrides.insert(field.clone(), value);
                }
            }
        }
    }

    // Apply minted JWT as Authorization header
    if let Some(jwt) = minted_jwt {
        injected_headers.insert("Authorization".to_string(), format!("Bearer {jwt}"));
    }

    // Apply body transforms
    let final_body_bytes = if body_overrides.is_empty() {
        body_bytes
    } else {
        apply_body_transforms(&body_bytes, &body_overrides)
    };

    // ── 7. Build upstream request ──────────────────────────────────────────
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(path);

    let upstream_url = match crate::proxy::router::Router::resolve_upstream(&matched_route, path_and_query) {
        Some(url) => url,
        None => {
            error!(request_id = %request_id, route_id = %route_id, "no upstream URL configured");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    info!(request_id = %request_id, upstream = %upstream_url, "proxying to upstream");

    let mut upstream_req = state.http_client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        &upstream_url,
    );

    // Forward non-hop-by-hop headers
    let mut fwd_headers = reqwest::header::HeaderMap::new();
    for (name, value) in &headers {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_str(name.as_str()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            fwd_headers.insert(n, v);
        }
    }

    // Inject hook-generated headers
    for (name, value) in &injected_headers {
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_str(name),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            fwd_headers.insert(n, v);
        }
    }

    // Set X-Request-Id
    if let Ok(v) = reqwest::header::HeaderValue::from_str(request_id) {
        fwd_headers.insert("x-request-id", v);
    }

    upstream_req = upstream_req.headers(fwd_headers);

    // Attach body
    if !final_body_bytes.is_empty() {
        upstream_req = upstream_req.body(final_body_bytes.to_vec());
    }

    // ── 8. Send upstream request ───────────────────────────────────────────
    let upstream_response = upstream_req.send().await.map_err(|e| {
        error!(request_id = %request_id, error = %e, upstream = %upstream_url, "upstream request failed");
        StatusCode::BAD_GATEWAY
    })?;

    let status = upstream_response.status();
    let resp_headers = upstream_response.headers().clone();

    // ── 9. Build response ──────────────────────────────────────────────────
    let mut response_builder = Response::builder()
        .status(status.as_u16());

    // Forward response headers
    for (name, value) in &resp_headers {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_str(name.as_str()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            response_builder = response_builder.header(n, v);
        }
    }

    // Always forward X-Request-Id
    response_builder = response_builder.header("x-request-id", request_id);

    // ── 10. Stream or buffer response ─────────────────────────────────────
    let stream_enabled = matched_route.config.stream.enabled;
    let request_start = std::time::Instant::now();

    let response_body = if stream_enabled {
        let stream_config = matched_route.config.stream.clone();

        // Extract user scopes for A2UI filtering
        let user_scopes: Vec<String> = identity
            .extra
            .get("a2ui_scopes")
            .map(|s| s.split(',').map(|sc| sc.trim().to_string()).collect())
            .unwrap_or_default();

        let mut processor = SseStreamProcessor::new(stream_config, user_scopes);
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
        // oneshot channel: streaming task sends total tokens when stream is done
        let (metrics_tx, metrics_rx) = tokio::sync::oneshot::channel::<u64>();

        tokio::spawn(async move {
            let mut byte_stream = upstream_response.bytes_stream();
            while let Some(chunk) = byte_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        match processor.process_chunk(&bytes) {
                            Some(processed) if !processed.is_empty() => {
                                if tx.send(Ok(processed)).await.is_err() {
                                    break;
                                }
                            }
                            None => {
                                // Backpressure limit hit — send SSE error and stop
                                let _ = tx
                                    .send(Ok(Bytes::from(
                                        "data: {\"type\":\"RUN_ERROR\",\"message\":\"stream limit exceeded\"}\n\n",
                                    )))
                                    .await;
                                break;
                            }
                            Some(_) => {} // empty processed chunk, skip
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
                            .await;
                        break;
                    }
                }
            }
            // Send total token count to the post-response task
            let _ = metrics_tx.send(processor.metrics().estimated_tokens);
        });

        // Post-response: wait for stream completion and log usage
        if let Some(db) = state.db.clone() {
            let user_id = identity.id.clone();
            let rid = request_id.to_string();
            let rid_clone = rid.clone();
            tokio::spawn(async move {
                let tokens = metrics_rx.await.unwrap_or(0);
                let duration_ms = request_start.elapsed().as_millis() as u64;
                let event = UsageEvent::new(rid_clone, user_id, route_id, tokens, duration_ms);
                if let Err(e) = db.log_usage(&event).await {
                    tracing::warn!(error = %e, request_id = %rid, "failed to log usage event");
                }
            });
        }

        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
    } else {
        let bytes = upstream_response.bytes().await.map_err(|e| {
            error!(request_id = %request_id, error = %e, "failed to read upstream response body");
            StatusCode::BAD_GATEWAY
        })?;
        Body::from(bytes)
    };

    response_builder
        .body(response_body)
        .map_err(|e| {
            error!(error = %e, "failed to build response");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// Apply body transform overrides to a JSON body.
///
/// Sets the specified fields in the JSON object. If the body is not valid JSON,
/// returns the original bytes unchanged.
fn apply_body_transforms(body: &Bytes, overrides: &HashMap<String, String>) -> Bytes {
    let mut value: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.clone(),
    };

    if let Value::Object(ref mut map) = value {
        for (field, val) in overrides {
            // Try to parse as JSON, fall back to string
            let json_val = serde_json::from_str(val)
                .unwrap_or_else(|_| Value::String(val.clone()));
            map.insert(field.clone(), json_val);
        }
    }

    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_transform_sets_field() {
        let body = Bytes::from(r#"{"model":"gpt-4","user":"old"}"#);
        let mut overrides = HashMap::new();
        overrides.insert("user".to_string(), "new-user".to_string());
        overrides.insert("temperature".to_string(), "0.7".to_string());

        let result = apply_body_transforms(&body, &overrides);
        let parsed: Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["user"], "new-user");
        assert_eq!(parsed["temperature"], 0.7);
        assert_eq!(parsed["model"], "gpt-4"); // unchanged
    }

    #[test]
    fn body_transform_non_json_passthrough() {
        let body = Bytes::from("plain text");
        let mut overrides = HashMap::new();
        overrides.insert("field".to_string(), "value".to_string());
        let result = apply_body_transforms(&body, &overrides);
        assert_eq!(result, Bytes::from("plain text"));
    }
}
