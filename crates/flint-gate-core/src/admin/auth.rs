//! Admin API authentication middleware.
//!
//! When `server.admin_auth` is configured, every admin request (except the
//! liveness/readiness probes, which are excluded at the router layer) must
//! authenticate against a reused [`Authenticator`] — typically JWT (Bearer) or
//! Kratos (session cookie), the Ory-standard path. There is no separate admin
//! identity model; the same verification primitives the proxy uses are reused
//! here.
//!
//! Fail-closed: any authenticator error maps to a 4xx/5xx and the request is
//! rejected. The resolved [`Identity`] is inserted into request extensions so
//! downstream handlers can attribute admin actions.

use crate::auth::{AuthError, Authenticator, Identity};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use std::sync::Arc;

/// Shared authenticator for the admin router. `None` means admin auth is not
/// configured (the router is only reachable on loopback in that case — enforced
/// by the startup posture guard, not here).
pub type AdminAuthenticator = Arc<dyn Authenticator>;

/// Map an [`AuthError`] to the admin API's `(status, message)`. Pure so the
/// fail-closed mapping is unit-testable without a live request.
///
/// - `Unauthorized`      → 401 (missing/invalid credential)
/// - `InsufficientScope` → 403 (verified but under-scoped)
/// - `ProviderError`     → 502 (auth backend unreachable) — still a rejection
/// - `NotConfigured`     → 401 (defensive: middleware only mounts when auth IS
///   configured, so reaching this means no credential resolved → deny)
pub fn admin_auth_status(err: &AuthError) -> (StatusCode, String) {
    match err {
        AuthError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
        AuthError::InsufficientScope { required } => (
            StatusCode::FORBIDDEN,
            format!("insufficient scope: requires {}", required.join(" ")),
        ),
        AuthError::ProviderError(msg) => (StatusCode::BAD_GATEWAY, msg.clone()),
        AuthError::NotConfigured => (
            StatusCode::UNAUTHORIZED,
            "admin authentication required".to_string(),
        ),
    }
}

/// Axum middleware: authenticate an admin request, attach the [`Identity`], or
/// reject. Mounted only when `server.admin_auth` is set.
pub async fn require_admin_auth(
    State(authenticator): State<AdminAuthenticator>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    match authenticator.authenticate(&parts).await {
        Ok(result) => {
            // Attach the resolved identity so handlers can attribute the action.
            parts.extensions.insert(result.identity);
            let request = Request::from_parts(parts, body);
            next.run(request).await
        }
        Err(err) => {
            let (status, message) = admin_auth_status(&err);
            (status, Json(json!({ "error": message }))).into_response()
        }
    }
}

/// Extract the authenticated admin [`Identity`] from request extensions, if the
/// auth middleware ran. `None` on a loopback-dev (no-auth) deployment.
#[allow(dead_code)]
pub fn admin_identity(parts: &axum::http::request::Parts) -> Option<&Identity> {
    parts.extensions.get::<Identity>()
}

#[cfg(test)]
mod tests {
    use super::admin_auth_status;
    use crate::auth::AuthError;
    use axum::http::StatusCode;

    #[test]
    fn unauthorized_maps_to_401() {
        let (status, msg) = admin_auth_status(&AuthError::Unauthorized("bad token".into()));
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "bad token");
    }

    #[test]
    fn insufficient_scope_maps_to_403() {
        let (status, msg) = admin_auth_status(&AuthError::InsufficientScope {
            required: vec!["admin".into(), "write".into()],
        });
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(msg.contains("admin write"));
    }

    #[test]
    fn provider_error_maps_to_502() {
        let (status, _) = admin_auth_status(&AuthError::ProviderError("kratos down".into()));
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn not_configured_still_denies_401() {
        // Defensive: the middleware only mounts when auth IS configured, so a
        // NotConfigured here means no credential resolved — must deny, never allow.
        let (status, _) = admin_auth_status(&AuthError::NotConfigured);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ── Middleware behavior over a real (minimal) router ──────────────────

    use crate::auth::{AuthMethod, AuthResult, Authenticator, Identity};
    use axum::{routing::get, Router};
    use http::request::Parts;
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    /// Stub authenticator that either accepts (with a fixed subject) or rejects.
    struct StubAuth {
        accept: bool,
    }

    #[async_trait::async_trait]
    impl Authenticator for StubAuth {
        async fn authenticate(&self, _parts: &Parts) -> Result<AuthResult, AuthError> {
            if self.accept {
                Ok(AuthResult {
                    identity: Identity::anonymous("admin-subject"),
                    method: AuthMethod::Anonymous,
                })
            } else {
                Err(AuthError::Unauthorized("no credential".into()))
            }
        }
    }

    /// Protected handler: echoes the attached identity id, proving the
    /// middleware ran and inserted the `Identity`.
    async fn protected(req: axum::extract::Request) -> String {
        let (parts, _) = req.into_parts();
        super::admin_identity(&parts)
            .map(|i| i.id.clone())
            .unwrap_or_else(|| "<none>".to_string())
    }

    fn app(accept: bool) -> Router {
        let auth: super::AdminAuthenticator = Arc::new(StubAuth { accept });
        Router::new()
            .route("/config", get(protected))
            .layer(axum::middleware::from_fn_with_state(
                auth,
                super::require_admin_auth,
            ))
    }

    #[tokio::test]
    async fn rejects_request_when_authenticator_denies() {
        let res = app(false)
            .oneshot(
                http::Request::builder()
                    .uri("/config")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn passes_and_attaches_identity_when_authenticator_accepts() {
        let res = app(true)
            .oneshot(
                http::Request::builder()
                    .uri("/config")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"admin-subject");
    }

    /// Mirror the real `admin_router_with_auth` composition — a public router
    /// (probes) merged with a protected router that owns a `.fallback(...)` and
    /// carries the auth `.layer(...)`. Proves the auth layer DOES cover the SPA
    /// static fallback (an unknown deep path), and that `/health` stays open.
    /// This closes the fail-open risk where an unauthenticated caller could
    /// reach the admin web UI via the fallback.
    fn composed_app(accept: bool) -> Router {
        let auth: super::AdminAuthenticator = Arc::new(StubAuth { accept });
        let public = Router::new().route("/health", get(|| async { "ok" }));
        let protected = Router::new()
            .route("/config", get(protected))
            .fallback(|| async { "spa-index" }) // stand-in for static_handler
            .layer(axum::middleware::from_fn_with_state(
                auth,
                super::require_admin_auth,
            ));
        public.merge(protected)
    }

    async fn status_of(app: Router, uri: &str) -> StatusCode {
        app.oneshot(
            http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    #[tokio::test]
    async fn spa_fallback_is_protected_when_auth_denies() {
        // An unknown deep path resolves to the protected router's fallback (the
        // SPA index in production). It MUST require auth — otherwise the admin
        // web UI is reachable unauthenticated.
        let st = status_of(composed_app(false), "/some/spa/route").await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "SPA fallback must be authed");
    }

    #[tokio::test]
    async fn health_probe_stays_open_in_composed_router() {
        let st = status_of(composed_app(false), "/health").await;
        assert_eq!(st, StatusCode::OK, "/health must not require auth");
    }

    #[tokio::test]
    async fn protected_and_fallback_pass_when_auth_accepts() {
        assert_eq!(status_of(composed_app(true), "/config").await, StatusCode::OK);
        assert_eq!(
            status_of(composed_app(true), "/some/spa/route").await,
            StatusCode::OK
        );
    }
}
