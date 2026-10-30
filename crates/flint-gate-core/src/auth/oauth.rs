//! Unified OAuth 2.0 token endpoint (`POST /oauth/token`) that dispatches by
//! `grant_type`, plus the RFC 7662 introspection endpoint. Both mount on the
//! proxy port. Each grant is independently gated by config; a grant that is not
//! enabled returns `unsupported_grant_type`.

use crate::auth::client_credentials::{
    client_credentials_grant, ClientCredentialsError, ClientCredentialsRequest,
    GRANT_TYPE_CLIENT_CREDENTIALS,
};
use crate::auth::introspect::{delegate_to_hydra, TokenVerifier};
use crate::auth::jwt_mint::SharedJwtMinter;
use crate::auth::token_exchange::{
    exchange_with_policy, ClientCredentials, ExchangePolicy, TokenExchangeRequest,
    GRANT_PARAMETERS, GRANT_TYPE_TOKEN_EXCHANGE,
};
use crate::auth::Authenticator;
use crate::db::Database;
use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// Token-exchange dependencies: the JWKS subject-token verifier, the delegated
/// token TTL, an optional Ory Hydra delegate (federate-first proxying), and the
/// client / audience policy for local minting.
pub type TokenExchangeDeps = (
    Arc<dyn Authenticator>,
    Option<u64>,
    Option<crate::auth::token_exchange::HydraDelegate>,
    ExchangePolicy,
);

/// State for the unified `/oauth/token` + `/oauth/introspect` endpoints. Each
/// capability is `Option` — absent means "not enabled".
#[derive(Clone)]
pub struct OAuthState {
    pub minter: SharedJwtMinter,
    /// Token-exchange config (RFC 8693). `None` disables that grant.
    pub token_exchange: Option<TokenExchangeDeps>,
    /// Client store + service-token TTL for client-credentials. `None` disables it.
    pub client_credentials: Option<(Arc<Database>, Option<u64>)>,
    /// Verifier + optional Hydra delegate for introspection. `None` disables it.
    pub introspection: Option<IntrospectionState>,
    /// Shared cross-replica rate limiter for `/oauth/token` (operator posture).
    /// `None` = no shared limiter configured (the in-process governor still
    /// applies as a per-replica burst shield).
    #[cfg(feature = "redis-l2")]
    pub token_limiter: Option<Arc<crate::ratelimit::OAuthLimiter>>,
    /// Shared cross-replica rate limiter for `/oauth/introspect` (always denies
    /// on a backend outage — the token-scanning oracle must not lose its limit).
    #[cfg(feature = "redis-l2")]
    pub introspect_limiter: Option<Arc<crate::ratelimit::OAuthLimiter>>,
}

/// Introspection dependencies.
#[derive(Clone)]
pub struct IntrospectionState {
    pub verifier: TokenVerifier,
    pub http_client: reqwest::Client,
    /// Hydra admin URL for delegating opaque-token introspection (seam, off unless set).
    pub hydra_admin_url: Option<String>,
    /// Require OAuth client authentication (RFC 7662 §2.1). When true, a request
    /// without valid client credentials is rejected before any introspection.
    pub require_auth: bool,
    /// Client store used to verify the caller's `client_id`/`client_secret`.
    /// Required when `require_auth` is true.
    pub client_store: Option<Arc<Database>>,
}

/// Extract OAuth client credentials from a request: HTTP Basic
/// (`Authorization: Basic base64(client_id:client_secret)`) OR the form fields
/// `client_id`/`client_secret` (RFC 6749 §2.3.1). Basic takes precedence. Pure
/// over the header value + form map so it is unit-testable.
pub fn extract_client_credentials(
    auth_header: Option<&str>,
    form: &HashMap<String, String>,
) -> Option<(String, String)> {
    if let Some(h) = auth_header {
        if let Some(b64) = h
            .strip_prefix("Basic ")
            .or_else(|| h.strip_prefix("basic "))
        {
            use base64::Engine;
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                if let Ok(decoded) = String::from_utf8(bytes) {
                    if let Some((id, secret)) = decoded.split_once(':') {
                        return Some((id.to_string(), secret.to_string()));
                    }
                }
            }
        }
    }
    match (form.get("client_id"), form.get("client_secret")) {
        (Some(id), Some(secret)) if !id.is_empty() => Some((id.clone(), secret.clone())),
        _ => None,
    }
}

fn oauth_error(status: StatusCode, code: &str, description: String) -> Response {
    // Never leak internal (e.g. database) error detail to the client on a 5xx —
    // log it server-side and return a generic description. Client (4xx) errors
    // keep their specific, safe message.
    let description = if status.is_server_error() {
        tracing::error!(error = %code, detail = %description, "oauth endpoint server error");
        "internal server error".to_string()
    } else {
        description
    };
    (
        status,
        Json(json!({ "error": code, "error_description": description })),
    )
        .into_response()
}

/// `POST /oauth/token` — dispatch by `grant_type`. The raw form is parsed once
/// into a map so the grant is selected before deserializing into the concrete
/// per-grant request type.
pub async fn token_endpoint(
    State(state): State<OAuthState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    // Shared cross-replica rate limit (when configured) — before any work.
    // Key by the authenticated client_id when present (Basic or form) so the
    // limit binds the client-credentials guessing surface even if the caller
    // rotates the raw Authorization header.
    #[cfg(feature = "redis-l2")]
    if let Some(limiter) = &state.token_limiter {
        let auth_header = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let client_id = extract_client_credentials(auth_header, &form).map(|c| c.0);
        if let Some(resp) = limiter
            .reject_response(&headers, client_id.as_deref())
            .await
        {
            return resp;
        }
    }

    let grant = form.get("grant_type").map(String::as_str).unwrap_or("");

    match grant {
        GRANT_TYPE_TOKEN_EXCHANGE => {
            let Some((verifier, ttl, delegate, policy)) = &state.token_exchange else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "unsupported_grant_type",
                    "token exchange is not enabled".into(),
                );
            };
            let auth_header = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok());
            let client_credentials =
                extract_client_credentials(auth_header, &form).map(|(client_id, client_secret)| {
                    ClientCredentials {
                        client_id,
                        client_secret,
                    }
                });
            // A `client_id` with no secret (and no HTTP Basic) names a public
            // client; whether it is one is decided by the exchange policy.
            let public_client_id = if client_credentials.is_none() && auth_header.is_none() {
                form.get("client_id")
                    .filter(|_| !form.contains_key("client_secret"))
                    .cloned()
            } else {
                None
            };
            let extra_fields = form
                .iter()
                .filter(|(k, _)| !GRANT_PARAMETERS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let req = TokenExchangeRequest {
                grant_type: grant.to_string(),
                subject_token: form.get("subject_token").cloned().unwrap_or_default(),
                subject_token_type: form.get("subject_token_type").cloned(),
                scope: form.get("scope").cloned(),
                resource: form.get("resource").cloned(),
                audience: form.get("audience").cloned(),
                actor_token: form.get("actor_token").cloned(),
                requested_token_type: form.get("requested_token_type").cloned(),
                requested_subject: form.get("requested_subject").cloned(),
                expires_in: form.get("expires_in").cloned(),
                client_credentials,
                public_client_id,
                extra_fields,
            };
            match exchange_with_policy(
                &req,
                verifier,
                &state.minter,
                *ttl,
                delegate.as_ref(),
                policy,
            )
            .await
            {
                Ok(body) => (StatusCode::OK, Json(body)).into_response(),
                Err(e) => oauth_error(e.http_status(), e.oauth_code(), e.message()),
            }
        }
        GRANT_TYPE_CLIENT_CREDENTIALS => {
            // A public token-exchange client may use only the session-token
            // exchange, even if an `oauth_clients` row with its id exists.
            let auth_header = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok());
            let presented_id = extract_client_credentials(auth_header, &form)
                .map(|(id, _)| id)
                .or_else(|| form.get("client_id").cloned());
            if let (Some(id), Some((_, _, _, policy))) = (presented_id, &state.token_exchange) {
                if policy.is_public_client(&id) {
                    return oauth_error(
                        StatusCode::BAD_REQUEST,
                        "unauthorized_client",
                        "a public client may not use client_credentials".into(),
                    );
                }
            }
            let Some((db, ttl)) = &state.client_credentials else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "unsupported_grant_type",
                    "client_credentials is not enabled".into(),
                );
            };
            let req = ClientCredentialsRequest {
                grant_type: grant.to_string(),
                client_id: form.get("client_id").cloned().unwrap_or_default(),
                client_secret: form.get("client_secret").cloned().unwrap_or_default(),
                scope: form.get("scope").cloned(),
            };
            match client_credentials_grant(&req, db, &state.minter, *ttl).await {
                Ok(body) => (StatusCode::OK, Json(body)).into_response(),
                Err(e) => {
                    let status = match &e {
                        ClientCredentialsError::MintFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
                        ClientCredentialsError::InvalidClient => StatusCode::UNAUTHORIZED,
                        _ => StatusCode::BAD_REQUEST,
                    };
                    oauth_error(status, e.oauth_code(), e.message())
                }
            }
        }
        other => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            format!("unsupported grant_type: {other}"),
        ),
    }
}

/// `POST /oauth/introspect` (RFC 7662). Verifies gateway-minted tokens locally;
/// when local verification reports inactive AND a Hydra delegate is configured,
/// forwards to Hydra (which owns introspection for its own opaque tokens).
pub async fn introspect_endpoint(
    State(state): State<OAuthState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    // Shared cross-replica rate limit (when configured) — the oracle always
    // denies on a backend outage; checked before any introspection work. Key by
    // the authenticated client_id when present so header rotation cannot mint
    // fresh windows against the token-scanning surface.
    #[cfg(feature = "redis-l2")]
    if let Some(limiter) = &state.introspect_limiter {
        let auth_header = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let client_id = extract_client_credentials(auth_header, &form).map(|c| c.0);
        if let Some(resp) = limiter
            .reject_response(&headers, client_id.as_deref())
            .await
        {
            return resp;
        }
    }

    let Some(intro) = &state.introspection else {
        return oauth_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "introspection is not enabled".into(),
        );
    };

    // RFC 7662 §2.1: authenticate the caller before answering. Fail-closed —
    // if auth is required, a missing/invalid client (or a missing store) is a
    // 401 BEFORE any token is inspected or forwarded to Hydra.
    if intro.require_auth {
        let auth_header = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let creds = extract_client_credentials(auth_header, &form);
        let authed = match (creds, &intro.client_store) {
            (Some((id, secret)), Some(db)) => matches!(
                db.verify_client_credentials(&id, &secret).await,
                Ok(Some(_))
            ),
            _ => false,
        };
        if !authed {
            return oauth_error(
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "client authentication required".into(),
            );
        }
    }

    let token = form.get("token").cloned().unwrap_or_default();
    let local = intro.verifier.introspect(&token);
    if local.get("active") == Some(&Value::Bool(true)) {
        return (StatusCode::OK, Json(local)).into_response();
    }
    // Local says inactive — try the Hydra delegate if configured.
    if let Some(hydra_url) = &intro.hydra_admin_url {
        let delegated = delegate_to_hydra(&intro.http_client, hydra_url, &token).await;
        return (StatusCode::OK, Json(delegated)).into_response();
    }
    (StatusCode::OK, Json(local)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn basic(id: &str, secret: &str) -> String {
        let raw = format!("{id}:{secret}");
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }

    // ── extract_client_credentials ────────────────────────────────────────

    #[test]
    fn extracts_from_basic_header() {
        let h = basic("svc-1", "s3cr3t");
        assert_eq!(
            extract_client_credentials(Some(&h), &HashMap::new()),
            Some(("svc-1".to_string(), "s3cr3t".to_string()))
        );
    }

    #[test]
    fn extracts_from_form_when_no_header() {
        let f = form(&[("client_id", "svc-2"), ("client_secret", "pw")]);
        assert_eq!(
            extract_client_credentials(None, &f),
            Some(("svc-2".to_string(), "pw".to_string()))
        );
    }

    #[test]
    fn basic_header_takes_precedence_over_form() {
        let h = basic("from-header", "hs");
        let f = form(&[("client_id", "from-form"), ("client_secret", "fs")]);
        assert_eq!(
            extract_client_credentials(Some(&h), &f),
            Some(("from-header".to_string(), "hs".to_string()))
        );
    }

    #[test]
    fn no_credentials_returns_none() {
        assert_eq!(extract_client_credentials(None, &HashMap::new()), None);
    }

    #[test]
    fn malformed_basic_falls_back_to_form_or_none() {
        // Non-base64 Basic header, no form creds → None (fail-closed).
        assert_eq!(
            extract_client_credentials(Some("Basic !!!notb64"), &HashMap::new()),
            None
        );
        // Base64 without a colon → None.
        let no_colon = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("nocolon")
        );
        assert_eq!(
            extract_client_credentials(Some(&no_colon), &HashMap::new()),
            None
        );
    }

    #[test]
    fn empty_client_id_in_form_is_rejected() {
        let f = form(&[("client_id", ""), ("client_secret", "pw")]);
        assert_eq!(extract_client_credentials(None, &f), None);
    }

    // ── introspect endpoint auth gate (router integration) ────────────────

    use axum::{routing::post, Router};
    use tower::ServiceExt;

    async fn introspect_app(require_auth: bool) -> Router {
        // require_auth=true with client_store=None: no request can authenticate,
        // so the gate must 401 everything BEFORE introspecting (fail-closed).
        // require_auth=false: introspection runs (garbage token → active:false).
        let cfg = crate::config::types::JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("oauth-endpoint-test-secret".into()),
            signing_key_path: None,
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
            ..Default::default()
        };
        let verifier = TokenVerifier::from_jwt_config(&cfg).await.unwrap();
        let state = OAuthState {
            minter: Arc::new(tokio::sync::RwLock::new(None)),
            token_exchange: None,
            client_credentials: None,
            introspection: Some(IntrospectionState {
                verifier,
                http_client: reqwest::Client::new(),
                hydra_admin_url: None,
                require_auth,
                client_store: None,
            }),
            #[cfg(feature = "redis-l2")]
            token_limiter: None,
            #[cfg(feature = "redis-l2")]
            introspect_limiter: None,
        };
        Router::new()
            .route("/oauth/introspect", post(introspect_endpoint))
            .with_state(state)
    }

    async fn post_introspect(app: Router, body: &str, auth: Option<&str>) -> StatusCode {
        let mut req = http::Request::builder()
            .method("POST")
            .uri("/oauth/introspect")
            .header("content-type", "application/x-www-form-urlencoded");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        app.oneshot(req.body(axum::body::Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn introspect_without_creds_is_401_when_auth_required() {
        let st = post_introspect(introspect_app(true).await, "token=abc", None).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn introspect_with_creds_but_no_store_still_401() {
        // Credentials present but no store to verify them → fail-closed 401.
        let h = basic("svc", "pw");
        let st = post_introspect(introspect_app(true).await, "token=abc", Some(&h)).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn introspect_runs_when_auth_not_required() {
        // auth disabled → endpoint answers (garbage token → 200 active:false).
        let st = post_introspect(introspect_app(false).await, "token=not.a.jwt", None).await;
        assert_eq!(st, StatusCode::OK);
    }

    // ── token endpoint: exchange policy through the form (router integration) ─

    struct OneSubject;

    #[async_trait::async_trait]
    impl Authenticator for OneSubject {
        async fn authenticate(
            &self,
            _parts: &http::request::Parts,
        ) -> Result<crate::auth::AuthResult, crate::auth::AuthError> {
            Ok(crate::auth::AuthResult {
                identity: crate::auth::identity::Identity {
                    id: "person-1".into(),
                    ..Default::default()
                },
                method: crate::auth::AuthMethod::BearerJwt,
            })
        }
    }

    struct OneClient;

    #[async_trait::async_trait]
    impl crate::auth::token_exchange::ClientVerifier for OneClient {
        async fn verify(
            &self,
            client_id: &str,
            client_secret: &str,
        ) -> anyhow::Result<Option<crate::auth::token_exchange::VerifiedClient>> {
            Ok(
                (client_id == "knowduet-gateway" && client_secret == "pw").then(|| {
                    crate::auth::token_exchange::VerifiedClient {
                        client_id: client_id.into(),
                        scopes: vec![],
                    }
                }),
            )
        }
    }

    async fn token_app() -> Router {
        let cfg = crate::config::types::JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("oauth-endpoint-test-secret".into()),
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
            ..Default::default()
        };
        let minter = crate::auth::jwt_mint::JwtMinter::from_config(&cfg)
            .await
            .unwrap();
        let mut policy = ExchangePolicy {
            client_verifier: Some(Arc::new(OneClient)),
            ..Default::default()
        };
        policy.clients.insert(
            "knowduet-gateway".into(),
            crate::config::types::ExchangeClientPolicy {
                requested_claims: vec!["topics".into()],
                ..Default::default()
            },
        );
        policy.clients.insert(
            "knowduet-desktop".into(),
            crate::config::types::ExchangeClientPolicy {
                public: true,
                session_token_audiences: vec!["knowduet-gateway".into()],
                ..Default::default()
            },
        );
        policy.session_verifier = Some(Arc::new(OneSession));
        let state = OAuthState {
            minter: Arc::new(tokio::sync::RwLock::new(Some(minter))),
            token_exchange: Some((Arc::new(OneSubject), None, None, policy)),
            client_credentials: None,
            introspection: None,
            #[cfg(feature = "redis-l2")]
            token_limiter: None,
            #[cfg(feature = "redis-l2")]
            introspect_limiter: None,
        };
        Router::new()
            .route("/oauth/token", post(token_endpoint))
            .with_state(state)
    }

    async fn post_token(body: &str, auth: Option<&str>) -> (StatusCode, Value) {
        let mut req = http::Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        let resp = token_app()
            .await
            .oneshot(req.body(axum::body::Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    const TICKET_FORM: &str =
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
        &subject_token=a.b.c&audience=fabric&expires_in=60\
        &topics=%5B%22user%3Aperson-1%22%5D";

    #[tokio::test]
    async fn token_endpoint_issues_a_ticket_to_an_authenticated_client() {
        let (status, body) = post_token(TICKET_FORM, Some(&basic("knowduet-gateway", "pw"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["expires_in"], 60);
        use base64::Engine;
        let payload = body["access_token"]
            .as_str()
            .unwrap()
            .split('.')
            .nth(1)
            .unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["topics"], serde_json::json!(["user:person-1"]));
        assert_eq!(claims["aud"], "fabric");
        assert_eq!(claims["sub"], "person-1");
    }

    #[tokio::test]
    async fn token_endpoint_accepts_form_client_authentication() {
        let body = format!("{TICKET_FORM}&client_id=knowduet-gateway&client_secret=pw");
        let (status, _) = post_token(&body, None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn token_endpoint_refuses_topics_without_client_authentication() {
        let (status, body) = post_token(TICKET_FORM, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_client");
    }

    #[tokio::test]
    async fn token_endpoint_refuses_a_wrong_client_secret() {
        let (status, _) = post_token(TICKET_FORM, Some(&basic("knowduet-gateway", "no"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// A Kratos stub that accepts the session token `sess`.
    struct OneSession;

    #[async_trait::async_trait]
    impl crate::auth::token_exchange::SessionTokenVerifier for OneSession {
        async fn verify_session_token(
            &self,
            session_token: &str,
        ) -> Result<crate::auth::identity::Identity, crate::auth::AuthError> {
            if session_token == "sess" {
                Ok(crate::auth::identity::Identity {
                    id: "identity-7".into(),
                    session_id: Some("s-1".into()),
                    ..Default::default()
                })
            } else {
                Err(crate::auth::AuthError::Unauthorized("no session".into()))
            }
        }
    }

    const SESSION_FORM: &str =
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
        &subject_token=sess&subject_token_type=urn%3Aory%3Akratos%3Asession-token\
        &audience=knowduet-gateway&client_id=knowduet-desktop";

    #[tokio::test]
    async fn token_endpoint_exchanges_a_session_token_for_a_public_client() {
        let (status, body) = post_token(SESSION_FORM, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let payload = body["access_token"]
            .as_str()
            .unwrap()
            .split('.')
            .nth(1)
            .unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["sub"], "identity-7");
        assert_eq!(claims["act"]["sub"], "knowduet-desktop");
        assert_eq!(claims["aud"], "knowduet-gateway");
    }

    #[tokio::test]
    async fn token_endpoint_refuses_a_bad_session_with_invalid_grant() {
        let body = SESSION_FORM.replace("subject_token=sess", "subject_token=nope");
        let (status, body) = post_token(&body, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_grant");
    }

    #[tokio::test]
    async fn token_endpoint_refuses_an_unknown_public_client_id() {
        let body = SESSION_FORM.replace("client_id=knowduet-desktop", "client_id=stranger");
        let (status, body) = post_token(&body, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_client");
    }

    #[tokio::test]
    async fn token_endpoint_refuses_client_credentials_for_a_public_client() {
        for (form, auth) in [
            (
                "grant_type=client_credentials&client_id=knowduet-desktop&client_secret=x",
                None,
            ),
            (
                "grant_type=client_credentials&client_id=knowduet-desktop",
                None,
            ),
            (
                "grant_type=client_credentials",
                Some(basic("knowduet-desktop", "x")),
            ),
        ] {
            let (status, body) = post_token(form, auth.as_deref()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{form}");
            assert_eq!(body["error"], "unauthorized_client", "{form}");
        }
    }
}
