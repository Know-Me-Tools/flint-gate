//! Envoy external-authorization check endpoint (`ext_authz`, HTTP service).
//!
//! Envoy Gateway's `SecurityPolicy.spec.extAuth.http` makes Envoy send a check
//! request to this endpoint before it routes a client request. The check
//! request copies the client's method, `Host` and the headers the policy
//! forwards, and its path is [`EXT_AUTHZ_PATH`] followed by the original path
//! (`extAuth.http.path` is a prefix). It has no body.
//!
//! The gate matches the original host and path against its own route table,
//! authenticates with that route's provider (`kratos`, `jwt`, `api_key`,
//! `anonymous`, ...) and answers:
//!
//! - **allow**: `200` with `Authorization: Bearer <gate-minted JWT>`, and
//!   `x-envoy-auth-headers-to-remove` naming the API-key headers. With
//!   `headersToBackend: ["authorization"]`, Envoy overwrites the client's
//!   `Authorization` with the minted bearer and removes the API-key headers,
//!   so the upstream sees only the gate's token.
//! - **deny**: `401` (no or invalid credential) or `403` (no gate route, a
//!   route this endpoint cannot evaluate, or insufficient scope), never a token.
//!
//! The check runs only authentication and minting. Routes with pre-request
//! hooks other than `claims_enhancement.mint_jwt` (Cedar `authorize`, budgets,
//! guardrails, ASO, injected headers) are denied rather than allowed with
//! their controls skipped.
use crate::auth::{AuthError, AuthMethod, AuthResult, Identity};
use crate::config::types::{AuthProviderConfig, MintJwtConfig, PreRequestHook};
use crate::config::{lookup::collect_hook_templates, TemplateContext, TemplateEngine};
use crate::middleware::AppState;
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Path prefix of the check endpoint. Set `extAuth.http.path` to this value.
pub const EXT_AUTHZ_PATH: &str = "/ext-authz";
/// Axum routes for the check endpoint: the bare prefix, the prefix plus the
/// root path, and the prefix plus any original path.
pub const EXT_AUTHZ_ROUTES: [&str; 3] =
    ["/ext-authz", "/ext-authz/", "/ext-authz/{*original_path}"];
/// Envoy reads this check-response header as a comma-separated list of
/// request headers to remove before forwarding upstream (applied after
/// `headersToBackend`, so it must never name `authorization`).
pub const ENVOY_HEADERS_TO_REMOVE: &str = "x-envoy-auth-headers-to-remove";
const DEFAULT_API_KEY_HEADER: &str = "x-api-key";

/// Answer one Envoy check request. Accepts any method because Envoy keeps the
/// client's method on the check request.
pub async fn ext_authz_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    let (parts, _body) = req.into_parts();
    let request_id = Uuid::new_v4().to_string();
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| parts.uri.host())
        .unwrap_or("")
        .to_string();
    let original_path = original_path(parts.uri.path()).to_string();

    let route = {
        let _snapshot = state.cache.configuration_snapshot_guard().await;
        let router = state.router.read().await;
        router
            .match_route(&host, &original_path, parts.method.as_str())
            .cloned()
    };
    let Some(route) = route else {
        info!(%request_id, %host, path = %original_path, "ext_authz deny: no route matched");
        return deny(StatusCode::FORBIDDEN, "no_route");
    };
    let route_id = route.config.id.as_str();
    let hooks = &route.config.hooks.pre_request;
    let Some(mint_cfg) = supported_mint_config(hooks) else {
        error!(%request_id, %route_id, "ext_authz deny: route has pre-request hooks the check endpoint does not evaluate");
        return deny(StatusCode::FORBIDDEN, "route_not_supported_by_ext_authz");
    };

    let provider = route
        .config
        .auth
        .as_deref()
        .or(route.site.default_auth.as_deref());
    let auth = match provider {
        None => anonymous(),
        Some(name) => {
            let Some(authenticator) = state.auth_providers.get(name) else {
                error!(%request_id, provider = %name, "ext_authz: configured auth provider not found");
                return deny(StatusCode::INTERNAL_SERVER_ERROR, "auth_provider_missing");
            };
            // Authority mode requires a fresh ASO decision for Kratos routes,
            // which this endpoint cannot make (mirrors the proxy pipeline).
            if state.cache.authority_readiness().is_some()
                && authenticator.kratos_cache_context().is_some()
            {
                error!(%request_id, %route_id, "ext_authz deny: authority-enabled Kratos route");
                return deny(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "authority_decision_unavailable",
                );
            }
            match authenticator.authenticate(&parts).await {
                Ok(result) => result,
                Err(AuthError::NotConfigured) => anonymous(),
                Err(AuthError::Unauthorized(reason)) => {
                    info!(%request_id, %route_id, provider = %name, %reason, "ext_authz deny: unauthorized");
                    return deny(StatusCode::UNAUTHORIZED, "unauthorized");
                }
                Err(AuthError::InsufficientScope { required }) => {
                    info!(%request_id, %route_id, provider = %name, ?required, "ext_authz deny: insufficient scope");
                    return deny(StatusCode::FORBIDDEN, "insufficient_scope");
                }
                Err(AuthError::ProviderError(reason)) => {
                    error!(%request_id, provider = %name, %reason, "ext_authz: auth provider error");
                    return deny(StatusCode::BAD_GATEWAY, "auth_provider_error");
                }
            }
        }
    };

    let claims = match mint_cfg {
        Some(cfg) => Some(render_claims(&state, cfg, hooks, &auth, &request_id).await),
        None => None,
    };
    let token = {
        let guard = state.jwt_minter.read().await;
        let Some(minter) = guard.as_ref() else {
            error!(%request_id, "ext_authz: JWT minting is not configured");
            return deny(StatusCode::SERVICE_UNAVAILABLE, "jwt_minting_unavailable");
        };
        match minter.mint(&auth.identity, claims.as_ref(), None) {
            Ok(token) => token,
            Err(e) => {
                error!(%request_id, error = %e, "ext_authz: JWT minting failed");
                return deny(StatusCode::INTERNAL_SERVER_ERROR, "jwt_minting_failed");
            }
        }
    };
    let Ok(bearer) = HeaderValue::from_str(&format!("Bearer {token}")) else {
        return deny(StatusCode::INTERNAL_SERVER_ERROR, "jwt_minting_failed");
    };
    let remove = api_key_headers(&state).await.join(",");
    let Ok(remove) = HeaderValue::from_str(&remove) else {
        return deny(StatusCode::INTERNAL_SERVER_ERROR, "invalid_api_key_header");
    };

    info!(%request_id, %route_id, user_id = %auth.identity.id, "ext_authz allow");
    let mut response = StatusCode::OK.into_response();
    let headers = response.headers_mut();
    headers.insert(header::AUTHORIZATION, bearer);
    headers.insert(ENVOY_HEADERS_TO_REMOVE, remove);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Strip the endpoint prefix from the check path to recover the client path.
fn original_path(check_path: &str) -> &str {
    match check_path.strip_prefix(EXT_AUTHZ_PATH) {
        Some("") | None => "/",
        Some(rest) => rest,
    }
}

/// `Some(mint)` when every hook is a `claims_enhancement` this endpoint can
/// honour (an optional `mint_jwt`, nothing else); `None` otherwise.
fn supported_mint_config(hooks: &[PreRequestHook]) -> Option<Option<&MintJwtConfig>> {
    let mut mint = None;
    for hook in hooks {
        match hook {
            PreRequestHook::ClaimsEnhancement { config }
                if config.inject_headers.is_empty() && config.aso_replica_grant.is_none() =>
            {
                if let Some(cfg) = config.mint_jwt.as_ref().filter(|c| c.enabled) {
                    mint = Some(cfg);
                }
            }
            _ => return None,
        }
    }
    Some(mint)
}

/// Render the route's `mint_jwt.additional_claims`, as the proxy pipeline does.
async fn render_claims(
    state: &AppState,
    cfg: &MintJwtConfig,
    hooks: &[PreRequestHook],
    auth: &AuthResult,
    request_id: &str,
) -> serde_json::Value {
    let mut api_key_ctx = HashMap::new();
    if let AuthMethod::ApiKey { client_id, scopes } = &auth.method {
        api_key_ctx.insert("client_id".to_string(), client_id.clone());
        api_key_ctx.insert("scopes".to_string(), scopes.join(","));
    }
    let mut ctx = TemplateContext::new(
        auth.identity.to_value(),
        serde_json::Value::Null,
        request_id.to_string(),
        api_key_ctx,
    );
    let templates = collect_hook_templates(hooks);
    let refs: Vec<&str> = templates.iter().map(String::as_str).collect();
    ctx.lookups = state.lookup_registry.resolve_all(&refs, &ctx).await;
    TemplateEngine::render_value(&cfg.additional_claims, &ctx)
}

/// Lower-cased API-key header names to remove upstream: `x-api-key` plus every
/// configured `api_key` provider header.
async fn api_key_headers(state: &AppState) -> Vec<String> {
    let config = state.config.read().await;
    let mut names: BTreeSet<String> = config
        .auth_providers
        .values()
        .filter_map(|p| match p {
            AuthProviderConfig::ApiKey(cfg) => Some(cfg.header.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    names.insert(DEFAULT_API_KEY_HEADER.to_string());
    names.into_iter().collect()
}

fn anonymous() -> AuthResult {
    AuthResult {
        identity: Identity::anonymous("anonymous"),
        method: AuthMethod::Anonymous,
    }
}

fn deny(status: StatusCode, error: &'static str) -> Response {
    if status.is_server_error() {
        warn!(%status, %error, "ext_authz check failed");
    }
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({ "error": error })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::ClaimsEnhancementConfig;

    #[test]
    fn original_path_strips_the_prefix() {
        assert_eq!(original_path("/ext-authz/api/chat"), "/api/chat");
        assert_eq!(original_path("/ext-authz"), "/");
        assert_eq!(original_path("/ext-authz/"), "/");
    }

    #[test]
    fn only_mint_claims_enhancement_is_supported() {
        let mint = PreRequestHook::ClaimsEnhancement {
            config: ClaimsEnhancementConfig {
                mint_jwt: Some(MintJwtConfig {
                    enabled: true,
                    additional_claims: serde_json::json!({"aud": "uar"}),
                }),
                ..Default::default()
            },
        };
        assert!(matches!(supported_mint_config(&[]), Some(None)));
        assert!(matches!(
            supported_mint_config(std::slice::from_ref(&mint)),
            Some(Some(_))
        ));

        let mut inject = ClaimsEnhancementConfig::default();
        inject
            .inject_headers
            .insert("x-user".into(), "{{ identity.id }}".into());
        let unsupported = PreRequestHook::ClaimsEnhancement { config: inject };
        assert!(supported_mint_config(&[mint, unsupported]).is_none());
    }
}
