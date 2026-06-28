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
use crate::auth::{AuthError, AuthMethod, Authenticator, Identity, SharedJwtMinter};
use crate::cache::GateCache;
use crate::config::{
    lookup::collect_hook_templates,
    types::{GateConfig, PostResponseHook, PreRequestHook},
    LookupRegistry, TemplateContext, TemplateEngine,
};
use crate::db::{Database, UsageEvent};
use crate::proxy::SharedRouter;
use crate::stream::{NdjsonStreamProcessor, SseStreamProcessor, StreamProcessor};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::StreamExt;
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
#[allow(dead_code)]
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

    // Extract the raw credential for cache key derivation (Authorization or Cookie).
    let raw_credential = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .or_else(|| parts.headers.get(http::header::COOKIE))
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let auth_result = if let Some(provider_name) = auth_provider_name {
        match state.auth_providers.get(provider_name) {
            Some(auth) => {
                // Fast path: check session cache before hitting the upstream auth provider.
                // Only cache Kratos-style session results; JWT and API key authenticators
                // manage their own caching internally.
                let cached = if let Some(ref cred) = raw_credential {
                    state.cache.get_session(cred).await
                } else {
                    None
                };

                if let Some(cached_identity) = cached {
                    info!(request_id = %request_id, user_id = %cached_identity.id, "session cache hit");
                    crate::auth::AuthResult {
                        identity: cached_identity,
                        method: AuthMethod::KratosSession,
                    }
                } else {
                    match auth.authenticate(&parts).await {
                        Ok(result) => {
                            // Populate session cache for Kratos results.
                            if matches!(result.method, AuthMethod::KratosSession) {
                                if let Some(ref cred) = raw_credential {
                                    state.cache.put_session(cred, &result.identity).await;
                                }
                            }
                            result
                        }
                        Err(AuthError::Unauthorized(msg)) => {
                            warn!(request_id = %request_id, provider = %provider_name, reason = %msg, "authentication failed");
                            return Err(StatusCode::UNAUTHORIZED);
                        }
                        Err(AuthError::ProviderError(msg)) => {
                            error!(request_id = %request_id, provider = %provider_name, error = %msg, "auth provider error");
                            return Err(StatusCode::BAD_GATEWAY);
                        }
                        Err(AuthError::NotConfigured) => crate::auth::AuthResult {
                            identity: Identity::anonymous("anonymous"),
                            method: AuthMethod::Anonymous,
                        },
                    }
                }
            }
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
        let template_refs: Vec<&str> = hook_templates.iter().map(String::as_str).collect();
        let resolved = state
            .lookup_registry
            .resolve_all(&template_refs, &template_ctx)
            .await;
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
            PreRequestHook::MaxTokenBudget { config } => {
                let user_id = TemplateEngine::render(
                    &format!("{{{{ {} }}}}", config.user_id_expr),
                    &template_ctx,
                );
                let lookup_key = format!("usage_budget({})", user_id);
                let used: u64 = template_ctx
                    .lookups
                    .get(&lookup_key)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if used >= config.limit {
                    let msg = config
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "token budget exceeded".to_string());
                    warn!(
                        request_id = %request_id,
                        user_id = %user_id,
                        used,
                        limit = config.limit,
                        "token budget exceeded — blocking request"
                    );
                    return Response::builder()
                        .status(StatusCode::TOO_MANY_REQUESTS)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({"error": "quota_exceeded", "message": msg})
                                .to_string(),
                        ))
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
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
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or(path);

    let upstream_url = match crate::proxy::router::Router::resolve_upstream(
        &matched_route,
        path_and_query,
    ) {
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
    let mut response_builder = Response::builder().status(status.as_u16());

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

        // Render AG-UI inject_metadata templates against the per-request context
        let ag_ui_metadata = {
            let mut map = serde_json::Map::new();
            for (key, template) in &stream_config.ai.ag_ui.inject_metadata {
                let rendered = TemplateEngine::render(template, &template_ctx);
                let value = serde_json::from_str::<serde_json::Value>(&rendered)
                    .unwrap_or(serde_json::Value::String(rendered));
                map.insert(key.clone(), value);
            }
            map
        };

        // A2UI theme from config
        let a2ui_theme = stream_config.ai.a2ui.theme.clone();

        // Protocol dispatch: create the appropriate processor
        let processor: Box<dyn StreamProcessor> = match stream_config.protocol.as_str() {
            "ndjson" => Box::new(NdjsonStreamProcessor::new(
                stream_config.clone(),
                user_scopes.clone(),
                ag_ui_metadata.clone(),
                a2ui_theme.clone(),
            )),
            _ => Box::new(SseStreamProcessor::new(
                stream_config.clone(),
                user_scopes.clone(),
                ag_ui_metadata.clone(),
                a2ui_theme.clone(),
            )),
        };

        // Session watchdog: spawn a periodic re-validation task when enabled
        let watchdog_cancel = tokio_util::sync::CancellationToken::new();
        if let Some(ref sw) = stream_config.ai.session_watchdog {
            if sw.enabled {
                let interval_secs = sw.check_interval_seconds;
                let credential = raw_credential.clone();
                let cache = state.cache.clone();
                let cancel = watchdog_cancel.clone();

                tokio::spawn(async move {
                    let mut ticker =
                        tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                    ticker.tick().await; // skip immediate first tick
                    loop {
                        tokio::select! {
                            _ = ticker.tick() => {
                                if let Some(ref cred) = credential {
                                    match cache.get_session(cred).await {
                                        Some(_) => { /* session still cached — valid */ }
                                        None => {
                                            tracing::warn!(
                                                "session watchdog: session no longer valid — terminating stream"
                                            );
                                            cancel.cancel();
                                            break;
                                        }
                                    }
                                }
                            }
                            _ = cancel.cancelled() => break,
                        }
                    }
                });
            }
        }

        let mut processor = processor;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
        // oneshot channel: streaming task sends total tokens when stream is done
        let (metrics_tx, metrics_rx) = tokio::sync::oneshot::channel::<u64>();

        let stream_cancel = watchdog_cancel.clone();
        let term_payload = processor.termination_payload();

        tokio::spawn(async move {
            let mut byte_stream = upstream_response.bytes_stream();
            loop {
                tokio::select! {
                    biased;
                    _ = stream_cancel.cancelled() => {
                        let _ = tx.send(Ok(Bytes::from(term_payload.clone()))).await;
                        break;
                    }
                    chunk = byte_stream.next() => {
                        match chunk {
                            Some(Ok(bytes)) => {
                                match processor.process_chunk(&bytes) {
                                    Some(processed) if !processed.is_empty() => {
                                        if tx.send(Ok(processed)).await.is_err() {
                                            break;
                                        }
                                    }
                                    None => {
                                        let _ = tx.send(Ok(Bytes::from(term_payload.clone()))).await;
                                        break;
                                    }
                                    Some(_) => {}
                                }
                            }
                            Some(Err(e)) => {
                                let _ = tx.send(Err(std::io::Error::other(e))).await;
                                break;
                            }
                            None => break,
                        }
                    }
                }
            }
            // Send total token count to the post-response task
            let _ = metrics_tx.send(processor.metrics().estimated_tokens);
        });

        // Post-response: log usage only when a StreamMeter hook with log_to_db=true is configured.
        let should_log = matched_route
            .config
            .hooks
            .post_response
            .iter()
            .any(|h| matches!(h, PostResponseHook::StreamMeter { config } if config.log_to_db));
        if should_log {
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
            } else {
                drop(metrics_rx);
            }
        } else {
            drop(metrics_rx);
        }

        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
    } else {
        let bytes = upstream_response.bytes().await.map_err(|e| {
            error!(request_id = %request_id, error = %e, "failed to read upstream response body");
            StatusCode::BAD_GATEWAY
        })?;
        Body::from(bytes)
    };

    response_builder.body(response_body).map_err(|e| {
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
            let json_val = serde_json::from_str(val).unwrap_or_else(|_| Value::String(val.clone()));
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
