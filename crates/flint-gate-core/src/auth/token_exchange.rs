//! OAuth 2.0 Token Exchange (RFC 8693) — gateway-local mode.
//!
//! `POST /oauth/token` with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.
//! flint-gate verifies the incoming `subject_token` against a configured JWKS
//! (so **any IdM that issues a verifiable JWT** — Ory Hydra is the reference —
//! is a valid subject-token source), **downscopes** the requested scope to a
//! subset of the subject token's scopes, and mints a delegated token carrying an
//! `act` (actor) claim via [`JwtMinter`]. It never forwards the subject token
//! upstream (confused-deputy defense).
//!
//! `delegate_to_hydra` (config) proxies the exchange to an Ory Hydra token
//! endpoint (federate-first) instead of minting locally — see [`HydraDelegate`]
//! and [`delegate_exchange_to_hydra`]. Local minting is the default.

use crate::auth::identity::Identity;
use crate::auth::jwt_mint::SharedJwtMinter;
use crate::auth::{AuthError, Authenticator};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// RFC 8693 grant type.
pub const GRANT_TYPE_TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
/// RFC 8693 token-type URN for an access token (the type we issue).
pub const TOKEN_TYPE_ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";

/// RFC 8693 request parameters (form-encoded). Only the fields we honor.
#[derive(Debug, Deserialize)]
pub struct TokenExchangeRequest {
    pub grant_type: String,
    pub subject_token: String,
    #[serde(default)]
    pub subject_token_type: Option<String>,
    /// Space-delimited requested scopes. Absent → inherit the subject's scopes.
    #[serde(default)]
    pub scope: Option<String>,
    /// RFC 8707 target resource / audience for the issued token.
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    /// Optional actor token (the agent acting on behalf of the subject).
    #[serde(default)]
    pub actor_token: Option<String>,
}

/// A structured token-exchange failure mapped to an OAuth 2.0 error response.
#[derive(Debug, PartialEq, Eq)]
pub enum ExchangeError {
    /// `grant_type` is not the token-exchange grant, or a required param is missing.
    UnsupportedGrantType(String),
    /// The `subject_token` failed verification (bad signature, expired, wrong iss/aud).
    InvalidSubjectToken(String),
    /// Requested scope is not a subset of the subject's scopes (escalation).
    InvalidScope(String),
    /// The exchange is not configured / disabled.
    NotEnabled,
    /// An `actor_token` was supplied but multi-hop delegation is not supported.
    /// Rejected fail-closed rather than silently ignored.
    UnsupportedActorToken,
    /// Minting the delegated token failed.
    MintFailed(String),
}

impl ExchangeError {
    /// The RFC 6749 §5.2 `error` code for this failure.
    pub fn oauth_code(&self) -> &'static str {
        match self {
            ExchangeError::UnsupportedGrantType(_) => "unsupported_grant_type",
            ExchangeError::InvalidSubjectToken(_) => "invalid_request",
            ExchangeError::InvalidScope(_) => "invalid_scope",
            ExchangeError::NotEnabled => "invalid_request",
            ExchangeError::UnsupportedActorToken => "invalid_request",
            ExchangeError::MintFailed(_) => "server_error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ExchangeError::UnsupportedGrantType(m)
            | ExchangeError::InvalidSubjectToken(m)
            | ExchangeError::InvalidScope(m)
            | ExchangeError::MintFailed(m) => m.clone(),
            ExchangeError::NotEnabled => "token exchange is not enabled".to_string(),
            ExchangeError::UnsupportedActorToken => {
                "actor_token (multi-hop delegation) is not supported".to_string()
            }
        }
    }
}

/// The identity + granted scopes resolved from a verified `subject_token`.
pub struct VerifiedSubject {
    pub identity: Identity,
    pub scopes: Vec<String>,
}

/// Extract the granted scopes from a verified subject identity. Handles both
/// conventions an IdM may emit in `metadata_public`:
/// - OAuth `scope`: a space-delimited **string** (`"read write"`).
/// - OIDC/AzureAD `scp`: either a space-delimited string OR a JSON **array**
///   of strings (`["read","write"]`).
///
/// Vendor-neutral by design — Ory, Auth0, Azure, and self-hosted issuers differ
/// here. Empty when neither claim is present. Never errors: an unrecognized
/// shape yields no scopes (fail-closed for the downscope subset check).
pub fn scopes_from_identity(identity: &Identity) -> Vec<String> {
    for key in ["scope", "scp"] {
        match identity.metadata_public.get(key) {
            Some(Value::String(s)) => {
                let scopes: Vec<String> = s.split_whitespace().map(str::to_string).collect();
                if !scopes.is_empty() {
                    return scopes;
                }
            }
            Some(Value::Array(items)) => {
                let scopes: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                if !scopes.is_empty() {
                    return scopes;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

/// Verify a `subject_token` by reusing the configured JWKS-backed
/// [`Authenticator`]. The token is presented via a synthetic `Authorization:
/// Bearer` header so the same verification path (JWKS fetch, signature,
/// iss/aud/exp) that guards the proxy is reused verbatim — the vendor-neutral
/// "any IdM with a verifiable JWT" guarantee.
///
/// Fail-closed: any verification error maps to [`ExchangeError::InvalidSubjectToken`].
pub async fn verify_subject_token(
    verifier: &Arc<dyn Authenticator>,
    subject_token: &str,
) -> Result<VerifiedSubject, ExchangeError> {
    let mut req = http::Request::new(());
    let bearer = format!("Bearer {subject_token}");
    let value = http::HeaderValue::from_str(&bearer)
        .map_err(|_| ExchangeError::InvalidSubjectToken("malformed subject_token".to_string()))?;
    req.headers_mut().insert(http::header::AUTHORIZATION, value);
    let (parts, _) = req.into_parts();

    match verifier.authenticate(&parts).await {
        Ok(result) => {
            let scopes = scopes_from_identity(&result.identity);
            Ok(VerifiedSubject {
                identity: result.identity,
                scopes,
            })
        }
        Err(AuthError::Unauthorized(msg)) => Err(ExchangeError::InvalidSubjectToken(msg)),
        Err(other) => Err(ExchangeError::InvalidSubjectToken(other.to_string())),
    }
}

/// Startup guard: validate that the configured `subject_token_provider` is a
/// JWKS-backed provider with its issuer pinned, so the exchange **cannot** be
/// wired to a fail-open verifier by misconfiguration.
///
/// Rejects (fail-closed):
/// - `anonymous` / `api_key` / `kratos` providers — these do not
///   cryptographically verify a bearer `subject_token` (an `anonymous` provider
///   would accept *any* string, minting a delegated token with no real subject).
/// - a `jwt` / `mcp` provider without a pinned `issuer` — an unpinned verifier
///   trusts any JWT its JWKS can validate, regardless of issuer/audience
///   (cross-issuer / cross-audience confused-deputy).
///
/// Returns `Err(reason)` describing the misconfiguration. Pure and unit-testable.
pub fn validate_subject_provider(
    provider: &crate::config::types::AuthProviderConfig,
) -> Result<(), String> {
    use crate::config::types::AuthProviderConfig as P;
    match provider {
        P::Jwt(cfg) => {
            if cfg.issuer.is_none() {
                return Err(
                    "token-exchange subject_token_provider (jwt) must pin `issuer` \
                     (and ideally `audience`); an unpinned JWT verifier trusts any \
                     issuer in its JWKS"
                        .to_string(),
                );
            }
            Ok(())
        }
        // The MCP provider already fails closed at build time without
        // issuer+audience (RFC 8707), so it is a safe subject-token verifier.
        P::Mcp(_) => Ok(()),
        P::Kratos(_) | P::ApiKey(_) | P::Anonymous(_) => Err(
            "token-exchange subject_token_provider must be a JWKS-backed `jwt` or \
             `mcp` provider that cryptographically verifies the subject_token; \
             anonymous/api_key/kratos providers would fail open"
                .to_string(),
        ),
    }
}

/// Validate the token-exchange request envelope (grant type + subject token
/// type). Pure so the RFC 8693 param validation is unit-testable.
pub fn validate_request(req: &TokenExchangeRequest) -> Result<(), ExchangeError> {
    if req.grant_type != GRANT_TYPE_TOKEN_EXCHANGE {
        return Err(ExchangeError::UnsupportedGrantType(format!(
            "unsupported grant_type: {}",
            req.grant_type
        )));
    }
    if req.subject_token.trim().is_empty() {
        return Err(ExchangeError::InvalidSubjectToken(
            "subject_token is required".to_string(),
        ));
    }
    // Fail-closed on an actor_token: multi-hop `act` chaining is not supported
    // this phase. Rejecting (rather than silently ignoring the security-relevant
    // parameter) prevents a caller from believing a delegation constraint was
    // applied when it was not.
    if req
        .actor_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return Err(ExchangeError::UnsupportedActorToken);
    }
    Ok(())
}

/// Downscope the requested scope to the subject token's granted scopes.
///
/// - No `requested` scope → the delegated token **inherits** the subject's scopes.
/// - `requested` present → every requested scope MUST be in `subject_scopes`,
///   otherwise the exchange is rejected (`InvalidScope`). This is the core
///   confused-deputy / privilege-escalation guard: a delegated token can only
///   ever carry a **subset** of what the subject already holds — never more.
///
/// Pure and fail-closed: an unknown/extra requested scope is a hard error, never
/// silently dropped. Returns the granted scope list for the delegated token.
pub fn downscope(
    requested: Option<&str>,
    subject_scopes: &[String],
) -> Result<Vec<String>, ExchangeError> {
    let Some(requested) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        // Inherit the subject's scopes verbatim.
        return Ok(subject_scopes.to_vec());
    };

    let mut granted = Vec::new();
    for scope in requested.split_whitespace() {
        if subject_scopes.iter().any(|s| s == scope) {
            granted.push(scope.to_string());
        } else {
            return Err(ExchangeError::InvalidScope(format!(
                "requested scope {scope:?} exceeds the subject token's granted scopes"
            )));
        }
    }
    Ok(granted)
}

/// Build the `additional_claims` object for the delegated token: the `act`
/// (actor) claim identifying the delegate, the downscoped `scope`, and the
/// RFC 8707 audience (`aud` from `resource`/`audience`). Pure so the claim shape
/// is unit-testable.
///
/// The `act` claim per RFC 8693 §4.1 nests the acting party; when the exchange
/// carries no distinct actor we record the subject as the delegate so the token
/// is always explicitly a delegated token, never indistinguishable from a
/// first-party one.
pub fn build_delegated_claims(
    subject: &Identity,
    granted_scopes: &[String],
    audience: Option<&str>,
    actor_sub: Option<&str>,
) -> Value {
    let act_sub = actor_sub.unwrap_or(&subject.id);
    let mut claims = json!({
        crate::auth::identity::FLINT_KIND_CLAIM: "agent",
        "act": { "sub": act_sub },
        "scope": granted_scopes.join(" "),
    });
    if let Some(aud) = audience {
        if let Value::Object(map) = &mut claims {
            map.insert("aud".to_string(), json!(aud));
        }
    }
    claims
}

/// The RFC 8693 §2.2.1 success response for an issued access token.
pub fn token_exchange_response(access_token: String, granted_scopes: &[String]) -> Value {
    json!({
        "access_token": access_token,
        "issued_token_type": TOKEN_TYPE_ACCESS_TOKEN,
        "token_type": "Bearer",
        "scope": granted_scopes.join(" "),
    })
}

/// State for the `POST /oauth/token` axum handler.
#[derive(Clone)]
pub struct TokenExchangeState {
    /// JWKS-backed verifier for the `subject_token` (resolved from the
    /// configured `subject_token_provider`).
    pub verifier: Arc<dyn Authenticator>,
    /// Shared minter for the delegated token.
    pub minter: SharedJwtMinter,
    /// TTL for minted delegated tokens.
    pub delegated_ttl_seconds: Option<u64>,
    /// Optional Ory Hydra delegate — when set, the exchange is proxied to Hydra.
    pub delegate: Option<HydraDelegate>,
}

/// `POST /oauth/token` (RFC 8693) axum handler. Maps [`ExchangeError`] to the
/// OAuth 2.0 error response (`400` for client errors, `500` for server errors).
pub async fn token_exchange_handler(
    axum::extract::State(state): axum::extract::State<TokenExchangeState>,
    axum::extract::Form(req): axum::extract::Form<TokenExchangeRequest>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    match exchange(
        &req,
        &state.verifier,
        &state.minter,
        state.delegated_ttl_seconds,
        state.delegate.as_ref(),
    )
    .await
    {
        Ok(body) => (StatusCode::OK, axum::response::Json(body)).into_response(),
        Err(e) => {
            let status = match &e {
                ExchangeError::MintFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_REQUEST,
            };
            (
                status,
                axum::response::Json(json!({
                    "error": e.oauth_code(),
                    "error_description": e.message(),
                })),
            )
                .into_response()
        }
    }
}

/// Configured Ory Hydra token-endpoint delegate for the exchange. When present,
/// the RFC 8693 request is proxied to Hydra (which owns 8693) instead of being
/// minted locally — the federate-first path.
#[derive(Clone)]
pub struct HydraDelegate {
    pub http_client: reqwest::Client,
    /// Hydra token endpoint, e.g. `https://hydra.example.com/oauth2/token`.
    pub token_url: String,
}

/// Proxy an RFC 8693 token-exchange request to a configured Ory Hydra token
/// endpoint. Fail-closed: a transport error or a non-2xx from Hydra is a
/// `MintFailed` (deny) — it never falls back to local minting, which would be a
/// confusing dual mode. Returns Hydra's JSON token response on success.
///
/// The `http_client` MUST be configured with `redirect(Policy::none())` — a
/// delegate posts the `subject_token` to a fixed operator URL, so following a
/// (compromised/misconfigured) Hydra 3xx to another host would exfiltrate the
/// token. A 3xx then surfaces as a non-2xx → fail-closed deny.
///
/// NOTE: Hydra has known `aud`-handling quirks when exchanging external tokens
/// (ory/hydra#3723); operators should validate their Hydra audience config.
pub async fn delegate_exchange_to_hydra(
    delegate: &HydraDelegate,
    req: &TokenExchangeRequest,
) -> Result<Value, ExchangeError> {
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", GRANT_TYPE_TOKEN_EXCHANGE),
        ("subject_token", req.subject_token.as_str()),
    ];
    if let Some(v) = req.subject_token_type.as_deref() {
        form.push(("subject_token_type", v));
    }
    if let Some(v) = req.scope.as_deref() {
        form.push(("scope", v));
    }
    if let Some(v) = req.resource.as_deref() {
        form.push(("resource", v));
    }
    if let Some(v) = req.audience.as_deref() {
        form.push(("audience", v));
    }

    // Observe every delegate outcome (result label) + round-trip latency, so
    // operators can see delegate volume and the share of tokens that bypass the
    // gateway's flint_kind agent-budget classification (see build-003 doc).
    let started = std::time::Instant::now();
    let record = |reason: &'static str| {
        crate::metrics::record_delegate(reason);
        crate::metrics::record_delegate_latency(started.elapsed().as_secs_f64());
    };

    let resp = match delegate
        .http_client
        .post(&delegate.token_url)
        .form(&form)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            record("deny_transport");
            return Err(ExchangeError::MintFailed(format!(
                "hydra token endpoint unreachable: {e}"
            )));
        }
    };

    if !resp.status().is_success() {
        // A 3xx surfaces here (the no-redirect client turns it into a non-2xx).
        record("deny_non2xx");
        return Err(ExchangeError::MintFailed(format!(
            "hydra token exchange returned {}",
            resp.status()
        )));
    }
    // Size-capped read (64 KiB) — a hostile/misbehaving Hydra cannot drive a
    // memory-pressure DoS; over-cap fails closed (deny).
    match crate::auth::http_body::read_capped_json(
        resp,
        crate::auth::http_body::MAX_UPSTREAM_BODY_BYTES,
    )
    .await
    {
        Ok(v) => {
            record("success");
            Ok(v)
        }
        Err(e) => {
            record("deny_badjson");
            Err(ExchangeError::MintFailed(format!(
                "hydra token response: {e}"
            )))
        }
    }
}

/// End-to-end token exchange: validate → (delegate to Hydra when configured, OR
/// verify `subject_token` → downscope → mint a delegated token via [`JwtMinter`]).
/// Returns the RFC 8693 response body or a mapped [`ExchangeError`].
///
/// Fail-closed at every step: a bad grant, an `actor_token`, an unverifiable
/// subject token, scope escalation, an unavailable minter, or a Hydra delegate
/// error all reject rather than issue a token.
pub async fn exchange(
    req: &TokenExchangeRequest,
    verifier: &Arc<dyn Authenticator>,
    minter: &SharedJwtMinter,
    delegated_ttl_seconds: Option<u64>,
    delegate: Option<&HydraDelegate>,
) -> Result<Value, ExchangeError> {
    // In delegate mode, a rejected actor_token is a delegate-path denial — record
    // it before returning so the metric captures every delegate outcome.
    if let Err(e) = validate_request(req) {
        if delegate.is_some() && matches!(e, ExchangeError::UnsupportedActorToken) {
            crate::metrics::record_delegate("deny_actor_token");
        }
        return Err(e);
    }

    // Federate-first: when a Hydra delegate is configured, proxy the exchange to
    // it (Hydra owns RFC 8693) instead of minting locally.
    if let Some(delegate) = delegate {
        return delegate_exchange_to_hydra(delegate, req).await;
    }

    let subject = verify_subject_token(verifier, &req.subject_token).await?;
    let granted = downscope(req.scope.as_deref(), &subject.scopes)?;

    // RFC 8707: prefer `resource`, then `audience`, as the issued token's `aud`.
    let audience = req.resource.as_deref().or(req.audience.as_deref());
    // A distinct verified actor is out of scope this change (single-hop `act`),
    // so the delegate is recorded as the subject itself — the token is still
    // explicitly a delegated token via the `act` claim.
    let additional = build_delegated_claims(&subject.identity, &granted, audience, None);

    let guard = minter.read().await;
    let minter = guard.as_ref().ok_or_else(|| {
        ExchangeError::MintFailed("JWT minter is not configured".to_string())
    })?;
    let token = minter
        .mint(&subject.identity, Some(&additional), delegated_ttl_seconds)
        .map_err(|e| ExchangeError::MintFailed(e.to_string()))?;

    Ok(token_exchange_response(token, &granted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(grant: &str, subject: &str) -> TokenExchangeRequest {
        TokenExchangeRequest {
            grant_type: grant.to_string(),
            subject_token: subject.to_string(),
            subject_token_type: None,
            scope: None,
            resource: None,
            audience: None,
            actor_token: None,
        }
    }

    #[test]
    fn validate_rejects_wrong_grant_type() {
        let err = validate_request(&req("client_credentials", "tok")).unwrap_err();
        assert_eq!(err.oauth_code(), "unsupported_grant_type");
    }

    #[test]
    fn validate_rejects_empty_subject_token() {
        let err = validate_request(&req(GRANT_TYPE_TOKEN_EXCHANGE, "   ")).unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_request");
    }

    #[test]
    fn validate_accepts_well_formed_request() {
        assert!(validate_request(&req(GRANT_TYPE_TOKEN_EXCHANGE, "a.b.c")).is_ok());
    }

    #[test]
    fn validate_rejects_actor_token_fail_closed() {
        // A present actor_token must be REJECTED (not silently ignored) — the
        // fail-closed close of the phase's silent-drop gap.
        let mut r = req(GRANT_TYPE_TOKEN_EXCHANGE, "a.b.c");
        r.actor_token = Some("actor.jwt.here".into());
        let err = validate_request(&r).unwrap_err();
        assert_eq!(err, ExchangeError::UnsupportedActorToken);
        assert_eq!(err.oauth_code(), "invalid_request");
    }

    #[test]
    fn validate_ignores_empty_actor_token() {
        // An empty/whitespace actor_token is not a real actor → allowed.
        let mut r = req(GRANT_TYPE_TOKEN_EXCHANGE, "a.b.c");
        r.actor_token = Some("   ".into());
        assert!(validate_request(&r).is_ok());
    }

    // ── Subject-provider startup guard (fail-closed against fail-open config) ─

    use crate::config::types::{
        AnonymousAuthConfig, AuthProviderConfig, JwtAuthConfig, KratosAuthConfig, McpAuthConfig,
    };

    #[test]
    fn subject_provider_rejects_anonymous() {
        // The exploit the security review found: an anonymous provider accepts
        // ANY subject_token → must be refused at startup.
        let p = AuthProviderConfig::Anonymous(AnonymousAuthConfig {
            default_subject: "anon".into(),
        });
        assert!(validate_subject_provider(&p).is_err());
    }

    #[test]
    fn subject_provider_rejects_kratos_and_api_key() {
        let kratos = AuthProviderConfig::Kratos(KratosAuthConfig {
            base_url: "http://kratos".into(),
            forward_cookies: true,
            session_cookie: "s".into(),
        });
        assert!(validate_subject_provider(&kratos).is_err());
    }

    #[test]
    fn subject_provider_rejects_jwt_without_pinned_issuer() {
        // An unpinned JWT verifier trusts any issuer in its JWKS → refuse.
        let p = AuthProviderConfig::Jwt(JwtAuthConfig {
            jwks_url: "https://idp/jwks".into(),
            issuer: None,
            audience: None,
            leeway_seconds: 5,
        });
        assert!(validate_subject_provider(&p).is_err());
    }

    #[test]
    fn subject_provider_accepts_issuer_pinned_jwt() {
        let p = AuthProviderConfig::Jwt(JwtAuthConfig {
            jwks_url: "https://idp/jwks".into(),
            issuer: Some("https://idp".into()),
            audience: Some("flint".into()),
            leeway_seconds: 5,
        });
        assert!(validate_subject_provider(&p).is_ok());
    }

    #[test]
    fn subject_provider_accepts_mcp() {
        // MCP already fails closed at build without issuer+audience (RFC 8707).
        let p = AuthProviderConfig::Mcp(McpAuthConfig {
            jwks_url: "https://as/jwks".into(),
            issuer: Some("https://as".into()),
            audience: Some("rs".into()),
            resource: "https://rs/mcp".into(),
            authorization_servers: vec![],
            required_scopes: vec![],
            leeway_seconds: 5,
        });
        assert!(validate_subject_provider(&p).is_ok());
    }

    #[test]
    fn scopes_parsed_from_metadata_scope_claim() {
        let identity = Identity {
            metadata_public: json!({ "scope": "read write admin" }),
            ..Default::default()
        };
        assert_eq!(scopes_from_identity(&identity), vec!["read", "write", "admin"]);
    }

    #[test]
    fn scopes_empty_when_claim_absent() {
        assert!(scopes_from_identity(&Identity::default()).is_empty());
    }

    #[test]
    fn scopes_parsed_from_scp_array_claim() {
        // OIDC/Azure-style `scp` as a JSON array.
        let identity = Identity {
            metadata_public: json!({ "scp": ["read", "write"] }),
            ..Default::default()
        };
        assert_eq!(scopes_from_identity(&identity), vec!["read", "write"]);
    }

    #[test]
    fn scopes_prefers_scope_string_then_scp() {
        // `scope` string wins when present; otherwise `scp` array is used.
        let with_scope = Identity {
            metadata_public: json!({ "scope": "admin", "scp": ["read"] }),
            ..Default::default()
        };
        assert_eq!(scopes_from_identity(&with_scope), vec!["admin"]);
    }

    // ── Scope downscoping (fail-closed escalation guard) ──────────────────

    fn subj() -> Vec<String> {
        vec!["read".into(), "write".into()]
    }

    #[test]
    fn downscope_inherits_subject_scopes_when_none_requested() {
        assert_eq!(downscope(None, &subj()).unwrap(), subj());
        assert_eq!(downscope(Some("   "), &subj()).unwrap(), subj());
    }

    #[test]
    fn downscope_allows_a_strict_subset() {
        assert_eq!(downscope(Some("read"), &subj()).unwrap(), vec!["read"]);
        assert_eq!(
            downscope(Some("read write"), &subj()).unwrap(),
            vec!["read", "write"]
        );
    }

    #[test]
    fn downscope_rejects_scope_escalation_fail_closed() {
        // The security crux: a requested scope the subject does NOT hold must be
        // rejected, never granted — a delegated token can only downscope.
        let err = downscope(Some("read admin"), &subj()).unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_scope");
        assert!(matches!(err, ExchangeError::InvalidScope(_)));
    }

    #[test]
    fn downscope_rejects_when_subject_has_no_scopes() {
        // Requesting anything against an empty subject scope set is escalation.
        let err = downscope(Some("read"), &[]).unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidScope(_)));
    }

    // ── Delegated-claim shape + response ──────────────────────────────────

    fn subject_identity() -> Identity {
        Identity {
            id: "user-42".into(),
            ..Default::default()
        }
    }

    #[test]
    fn delegated_claims_include_act_and_scope() {
        let claims = build_delegated_claims(&subject_identity(), &["read".into()], None, None);
        // act.sub defaults to the subject when no distinct actor is present.
        assert_eq!(claims["act"]["sub"], "user-42");
        assert_eq!(claims["scope"], "read");
        // The gateway stamps flint_kind=agent so the delegated token authorizes
        // as an Agent on re-verification.
        assert_eq!(claims["flint_kind"], "agent");
        assert!(claims.get("aud").is_none());
    }

    #[test]
    fn delegated_claims_bind_audience_when_present() {
        let claims = build_delegated_claims(
            &subject_identity(),
            &["read".into(), "write".into()],
            Some("https://api.example.com"),
            Some("agent-7"),
        );
        assert_eq!(claims["aud"], "https://api.example.com");
        assert_eq!(claims["act"]["sub"], "agent-7");
        assert_eq!(claims["scope"], "read write");
    }

    #[test]
    fn response_has_rfc8693_shape() {
        let resp = token_exchange_response("tok.abc".into(), &["read".into()]);
        assert_eq!(resp["access_token"], "tok.abc");
        assert_eq!(resp["token_type"], "Bearer");
        assert_eq!(resp["issued_token_type"], TOKEN_TYPE_ACCESS_TOKEN);
        assert_eq!(resp["scope"], "read");
    }

    // ── End-to-end exchange() (verify → downscope → mint) ─────────────────

    use crate::auth::jwt_mint::JwtMinter;
    use crate::auth::{AuthMethod, AuthResult};
    use crate::config::types::JwtConfig;
    use http::request::Parts;
    use tokio::sync::RwLock;

    /// Stub subject-token verifier: models "any IdM that issued a verifiable
    /// JWT". `accept=false` simulates a bad/expired/wrong-issuer token.
    struct StubVerifier {
        accept: bool,
        scopes: &'static str,
    }

    #[async_trait::async_trait]
    impl Authenticator for StubVerifier {
        async fn authenticate(&self, _parts: &Parts) -> Result<AuthResult, AuthError> {
            if self.accept {
                Ok(AuthResult {
                    identity: Identity {
                        id: "subject-user".into(),
                        metadata_public: json!({ "scope": self.scopes }),
                        ..Default::default()
                    },
                    method: AuthMethod::BearerJwt,
                })
            } else {
                Err(AuthError::Unauthorized("subject token invalid".into()))
            }
        }
    }

    async fn minter() -> SharedJwtMinter {
        let cfg = JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("test-secret-key-minimum-length".into()),
            signing_key_path: None,
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
        };
        Arc::new(RwLock::new(Some(
            JwtMinter::from_config(&cfg).await.unwrap(),
        )))
    }

    fn exchange_req(scope: Option<&str>) -> TokenExchangeRequest {
        TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "any.verifiable.jwt".into(),
            subject_token_type: None,
            scope: scope.map(str::to_string),
            resource: Some("https://api.example.com".into()),
            audience: None,
            actor_token: None,
        }
    }

    #[tokio::test]
    async fn exchange_succeeds_with_downscoped_delegated_token() {
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read write admin",
        });
        let m = minter().await;
        let resp = exchange(&exchange_req(Some("read write")), &verifier, &m, None, None)
            .await
            .expect("valid exchange");
        assert_eq!(resp["scope"], "read write");
        assert_eq!(resp["token_type"], "Bearer");
        // A JWT was minted.
        assert_eq!(resp["access_token"].as_str().unwrap().split('.').count(), 3);
    }

    #[tokio::test]
    async fn exchange_accepts_subject_token_from_any_jwks_idm() {
        // The verifier stands in for ANY JWKS-backed IdM (Ory Hydra, Auth0, a
        // self-hosted issuer…). As long as the subject token verifies, the
        // exchange issues a delegated token — the vendor-neutral guarantee.
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        assert!(exchange(&exchange_req(None), &verifier, &m, None, None).await.is_ok());
    }

    #[tokio::test]
    async fn exchange_denies_scope_escalation() {
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(Some("read admin")), &verifier, &m, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_scope");
    }

    #[tokio::test]
    async fn exchange_denies_invalid_subject_token() {
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: false,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidSubjectToken(_)));
    }

    #[tokio::test]
    async fn exchange_degrades_to_deny_on_wrong_grant_type() {
        // Malformed / wrong-grant input must reject before any token is minted.
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let mut bad = exchange_req(None);
        bad.grant_type = "authorization_code".into();
        let err = exchange(&bad, &verifier, &m, None, None).await.unwrap_err();
        assert_eq!(err.oauth_code(), "unsupported_grant_type");
    }

    #[tokio::test]
    async fn exchange_rejects_actor_token_before_minting() {
        // An actor_token present → rejected, no token minted (fail-closed).
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let mut req = exchange_req(None);
        req.actor_token = Some("actor.jwt".into());
        let err = exchange(&req, &verifier, &m, None, None).await.unwrap_err();
        assert_eq!(err, ExchangeError::UnsupportedActorToken);
    }

    // ── Hydra-delegate exchange ───────────────────────────────────────────

    #[tokio::test]
    async fn delegate_forwards_to_hydra_and_returns_its_token() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth2/token"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": "hydra-minted-token",
                    "token_type": "Bearer",
                    "scope": "read"
                })),
            )
            .mount(&server)
            .await;

        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: format!("{}/oauth2/token", server.uri()),
        };
        // With a delegate set, exchange proxies to Hydra — the local verifier/
        // minter are NOT consulted (a deny-all verifier proves it's bypassed).
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: false,
            scopes: "",
        });
        let m = minter().await;
        let resp = exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .expect("delegate exchange succeeds");
        assert_eq!(resp["access_token"], "hydra-minted-token");
    }

    #[tokio::test]
    async fn delegate_success_is_metered_and_rendered() {
        // A successful delegate exchange records into the global recorder that
        // the admin /metrics endpoint renders — proving the delegate→metric→
        // render chain end-to-end (the endpoint itself is admin-router only).
        crate::metrics::install_recorder();
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "t", "token_type": "Bearer" })),
            )
            .mount(&server)
            .await;
        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: format!("{}/oauth2/token", server.uri()),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: false,
            scopes: "",
        });
        let m = minter().await;
        exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .expect("delegate exchange succeeds");
        let out = crate::metrics::render();
        assert!(
            out.contains("flint_delegate_total") && out.contains("result=\"success\""),
            "delegate success not metered:\n{out}"
        );
    }

    #[tokio::test]
    async fn delegate_fails_closed_on_oversized_hydra_body() {
        // A 200 with a body larger than the 64 KiB cap must fail closed (deny),
        // not buffer unbounded (memory-pressure DoS guard).
        let server = wiremock::MockServer::start().await;
        let huge = format!(r#"{{"access_token":"{}"}}"#, "A".repeat(70 * 1024));
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(huge))
            .mount(&server)
            .await;

        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: format!("{}/oauth2/token", server.uri()),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::MintFailed(_)));
    }

    #[tokio::test]
    async fn delegate_fails_closed_on_hydra_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: format!("{}/oauth2/token", server.uri()),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .unwrap_err();
        // Non-2xx from Hydra → MintFailed (deny), never a local-mint fallback.
        assert!(matches!(err, ExchangeError::MintFailed(_)));
    }

    #[tokio::test]
    async fn delegate_fails_closed_on_hydra_redirect() {
        // A 3xx from Hydra must NOT be followed (token-exfiltration guard). With
        // a no-redirect client the 302 surfaces as a non-2xx → deny.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(302)
                    .insert_header("location", "https://attacker.example/steal"),
            )
            .mount(&server)
            .await;

        let delegate = HydraDelegate {
            http_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            token_url: format!("{}/oauth2/token", server.uri()),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::MintFailed(_)));
    }

    #[tokio::test]
    async fn delegate_fails_closed_on_transport_error() {
        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: "http://127.0.0.1:1/oauth2/token".into(),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, Some(&delegate))
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::MintFailed(_)));
    }
}
