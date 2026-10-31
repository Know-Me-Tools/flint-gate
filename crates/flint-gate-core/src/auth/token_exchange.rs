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
//!
//! Local minting also honors an [`ExchangePolicy`]:
//! - **Client authentication** (optional): HTTP Basic or `client_id` /
//!   `client_secret` form fields, verified against the `oauth_clients` store.
//!   An authenticated client is recorded as `act.sub`.
//! - **On-behalf-of** (`requested_subject`, no `subject_token`): an
//!   authenticated client allow-listed in `token_exchange.clients` obtains a
//!   token naming a person for one of its permitted audiences.
//! - **Requested claims**: an allow-listed client sets named claims (for
//!   example `topics`) through form fields of the same name.
//! - **Per-audience claims**: claims copied from the subject token and fixed
//!   claims, configured per issued `aud` in `token_exchange.audiences`.
//! - **Requested lifetime** (`expires_in`): shortens, never lengthens, the
//!   issued token's lifetime. The response always carries `expires_in`.
//! - **Kratos session-token exchange** (public clients): a client marked
//!   `public` (no secret, identified by the `client_id` form field alone)
//!   presents a Kratos session token as `subject_token` with
//!   `subject_token_type` = [`TOKEN_TYPE_KRATOS_SESSION_TOKEN`]. The gate
//!   verifies it with Kratos `/sessions/whoami` and mints a token for one of
//!   the client's `session_token_audiences`, `sub` = the Kratos identity,
//!   `act.sub` = the client. A public client can do nothing else.

use crate::auth::identity::Identity;
use crate::auth::jwt_mint::SharedJwtMinter;
use crate::auth::{AuthError, Authenticator};
use crate::config::types::{ExchangeAudiencePolicy, ExchangeClientPolicy, TokenExchangeConfig};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// RFC 8693 grant type.
pub const GRANT_TYPE_TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
/// RFC 8693 token-type URN for an access token (the type we issue).
pub const TOKEN_TYPE_ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";
/// `subject_token_type` for an Ory Kratos session token (the `session_token`
/// a native login flow returns). Not an IANA-registered URN: RFC 8693 §3
/// allows deployment-defined token-type URIs.
pub const TOKEN_TYPE_KRATOS_SESSION_TOKEN: &str = "urn:ory:kratos:session-token";

/// Form parameters the token-exchange grant itself interprets. None of them can
/// be configured as a requested claim.
pub const GRANT_PARAMETERS: &[&str] = &[
    "grant_type",
    "subject_token",
    "subject_token_type",
    "scope",
    "resource",
    "audience",
    "actor_token",
    "actor_token_type",
    "requested_token_type",
    "requested_subject",
    "expires_in",
    "client_id",
    "client_secret",
];

/// Claims the gateway owns on an exchanged token. Neither per-audience claims
/// nor requested claims can set them.
pub const EXCHANGE_RESERVED_CLAIMS: &[&str] = &[
    "iss",
    "sub",
    "aud",
    "exp",
    "iat",
    "nbf",
    "jti",
    "act",
    "scope",
    "client_id",
    crate::auth::identity::FLINT_KIND_CLAIM,
];

/// Whether `name` is a gateway-owned claim on an exchanged token.
pub fn is_exchange_reserved_claim(name: &str) -> bool {
    EXCHANGE_RESERVED_CLAIMS.contains(&name)
}

/// OAuth client credentials presented on the exchange. `Debug` never prints
/// the secret.
#[derive(Clone, Default)]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCredentials")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// RFC 8693 request parameters (form-encoded). Only the fields we honor.
#[derive(Debug, Default, Deserialize)]
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
    /// RFC 8693 `requested_token_type`. Checked on the on-behalf-of path, where
    /// only the access-token type is issued.
    #[serde(default)]
    pub requested_token_type: Option<String>,
    /// On-behalf-of: the person the issued token names (`sub`). Needs an
    /// authenticated, allow-listed client and no `subject_token`.
    #[serde(default)]
    pub requested_subject: Option<String>,
    /// Requested lifetime in seconds. Only shortens: the issued lifetime is the
    /// smaller of this and the configured exchange TTL.
    #[serde(default)]
    pub expires_in: Option<String>,
    /// Client credentials (HTTP Basic or form), filled in by the token endpoint.
    #[serde(skip)]
    pub client_credentials: Option<ClientCredentials>,
    /// A `client_id` form field sent without a secret (and without HTTP Basic):
    /// a public client's identifier. Filled in by the token endpoint.
    #[serde(skip)]
    pub public_client_id: Option<String>,
    /// Form fields that are not [`GRANT_PARAMETERS`]: requested-claim candidates.
    #[serde(skip)]
    pub extra_fields: BTreeMap<String, String>,
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
    /// A malformed or unsupported parameter combination.
    InvalidRequest(String),
    /// Client authentication was required, missing, or failed.
    InvalidClient,
    /// The authenticated client is not permitted to do what it asked.
    UnauthorizedClient(String),
    /// The requested audience is not permitted for this client.
    InvalidTarget(String),
    /// The presented grant (a Kratos session token) is invalid or expired, or
    /// could not be verified.
    InvalidGrant(String),
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
            ExchangeError::InvalidRequest(_) => "invalid_request",
            ExchangeError::InvalidClient => "invalid_client",
            ExchangeError::UnauthorizedClient(_) => "unauthorized_client",
            ExchangeError::InvalidTarget(_) => "invalid_target",
            ExchangeError::InvalidGrant(_) => "invalid_grant",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ExchangeError::UnsupportedGrantType(m)
            | ExchangeError::InvalidSubjectToken(m)
            | ExchangeError::InvalidScope(m)
            | ExchangeError::MintFailed(m)
            | ExchangeError::InvalidRequest(m)
            | ExchangeError::UnauthorizedClient(m)
            | ExchangeError::InvalidTarget(m)
            | ExchangeError::InvalidGrant(m) => m.clone(),
            ExchangeError::InvalidClient => "invalid client credentials".to_string(),
            ExchangeError::NotEnabled => "token exchange is not enabled".to_string(),
            ExchangeError::UnsupportedActorToken => {
                "actor_token (multi-hop delegation) is not supported".to_string()
            }
        }
    }

    /// The HTTP status for this failure: `401` for client authentication and
    /// an unverifiable session token, `500` for a mint failure, otherwise `400`.
    pub fn http_status(&self) -> http::StatusCode {
        match self {
            ExchangeError::MintFailed(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
            ExchangeError::InvalidClient | ExchangeError::InvalidGrant(_) => {
                http::StatusCode::UNAUTHORIZED
            }
            _ => http::StatusCode::BAD_REQUEST,
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
/// `expires_in` is the issued token's lifetime in seconds.
pub fn token_exchange_response(
    access_token: String,
    granted_scopes: &[String],
    expires_in: u64,
) -> Value {
    json!({
        "access_token": access_token,
        "issued_token_type": TOKEN_TYPE_ACCESS_TOKEN,
        "token_type": "Bearer",
        "expires_in": expires_in,
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
        Err(e) => (
            e.http_status(),
            axum::response::Json(json!({
                "error": e.oauth_code(),
                "error_description": e.message(),
            })),
        )
            .into_response(),
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

/// A client authenticated against the `oauth_clients` store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClient {
    pub client_id: String,
    /// The client's granted scopes (the ceiling for on-behalf-of tokens).
    pub scopes: Vec<String>,
}

/// Verifies OAuth client credentials. Implemented by the database client store;
/// a trait so the exchange is testable without Postgres.
#[async_trait::async_trait]
pub trait ClientVerifier: Send + Sync {
    /// `Ok(None)` for an unknown client or a wrong secret.
    async fn verify(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> anyhow::Result<Option<VerifiedClient>>;
}

#[async_trait::async_trait]
impl ClientVerifier for crate::db::Database {
    async fn verify(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> anyhow::Result<Option<VerifiedClient>> {
        Ok(self
            .verify_client_credentials(client_id, client_secret)
            .await?
            .map(|record| VerifiedClient {
                client_id: record.client_id,
                scopes: record.scopes,
            }))
    }
}

/// Verifies a Kratos session token and returns the session's identity.
/// Implemented by [`KratosAuthenticator`]; a trait so the exchange is testable
/// without Kratos.
///
/// [`KratosAuthenticator`]: crate::auth::kratos::KratosAuthenticator
#[async_trait::async_trait]
pub trait SessionTokenVerifier: Send + Sync {
    async fn verify_session_token(&self, session_token: &str) -> Result<Identity, AuthError>;
}

#[async_trait::async_trait]
impl SessionTokenVerifier for crate::auth::kratos::KratosAuthenticator {
    async fn verify_session_token(&self, session_token: &str) -> Result<Identity, AuthError> {
        self.authenticate_session_token(session_token)
            .await
            .map(|result| result.identity)
    }
}

/// Client and audience policy for gateway-local exchange. The default (empty)
/// policy reproduces the plain RFC 8693 exchange: no on-behalf-of, no requested
/// claims, no per-audience claims.
#[derive(Clone, Default)]
pub struct ExchangePolicy {
    pub clients: HashMap<String, ExchangeClientPolicy>,
    pub audiences: HashMap<String, ExchangeAudiencePolicy>,
    /// Verifies presented client credentials. `None`: any presented client
    /// credentials are refused (`invalid_client`), never ignored.
    pub client_verifier: Option<Arc<dyn ClientVerifier>>,
    /// Verifies Kratos session tokens for public clients. `None`: the
    /// session-token exchange is refused.
    pub session_verifier: Option<Arc<dyn SessionTokenVerifier>>,
}

impl ExchangePolicy {
    /// The policy from config, with `client_verifier` left to the caller.
    pub fn from_config(cfg: &TokenExchangeConfig) -> Self {
        Self {
            clients: cfg.clients.clone(),
            audiences: cfg.audiences.clone(),
            client_verifier: None,
            session_verifier: None,
        }
    }

    /// Whether `client_id` is configured as a public client.
    pub fn is_public_client(&self, client_id: &str) -> bool {
        self.clients.get(client_id).is_some_and(|p| p.public)
    }

    /// Whether `name` is a requested claim for any configured client.
    fn is_requestable_claim(&self, name: &str) -> bool {
        self.clients
            .values()
            .any(|p| p.requested_claims.iter().any(|c| c == name))
    }
}

/// Startup guard for `token_exchange.clients` / `token_exchange.audiences`.
/// Refuses reserved claim names, claim names that collide with grant
/// parameters, empty names, and (with `delegate_to_hydra`) any policy at all,
/// since the delegate path mints nothing locally. Pure and unit-testable.
pub fn validate_exchange_policy(cfg: &TokenExchangeConfig) -> Result<(), String> {
    if cfg.delegate_to_hydra && (!cfg.clients.is_empty() || !cfg.audiences.is_empty()) {
        return Err(
            "token_exchange.clients and token_exchange.audiences apply to local minting only; \
             they cannot be combined with delegate_to_hydra"
                .to_string(),
        );
    }
    for (client_id, policy) in &cfg.clients {
        if client_id.trim().is_empty() {
            return Err("token_exchange.clients has an empty client_id".to_string());
        }
        for aud in &policy.on_behalf_of_audiences {
            if aud.trim().is_empty() {
                return Err(format!(
                    "token_exchange.clients.{client_id}.on_behalf_of_audiences has an empty audience"
                ));
            }
        }
        for name in &policy.requested_claims {
            if name.trim().is_empty()
                || is_exchange_reserved_claim(name)
                || GRANT_PARAMETERS.contains(&name.as_str())
            {
                return Err(format!(
                    "token_exchange.clients.{client_id}.requested_claims: {name:?} is a reserved \
                     claim or grant parameter and cannot be requested"
                ));
            }
        }
        validate_public_client(client_id, policy, cfg.session_token_provider.is_some())?;
    }
    for (aud, policy) in &cfg.audiences {
        let names = policy
            .copy_claims
            .iter()
            .map(String::as_str)
            .chain(policy.claims.keys().map(String::as_str));
        for name in names {
            if name.trim().is_empty() || is_exchange_reserved_claim(name) {
                return Err(format!(
                    "token_exchange.audiences.{aud}: {name:?} is a reserved claim and cannot be set"
                ));
            }
        }
    }
    Ok(())
}

/// A public client may use only the session-token exchange: no on-behalf-of,
/// no requested claims, at least one session-token audience, and a configured
/// `session_token_provider`. A confidential client may not list
/// session-token audiences.
fn validate_public_client(
    client_id: &str,
    policy: &ExchangeClientPolicy,
    has_session_provider: bool,
) -> Result<(), String> {
    let prefix = format!("token_exchange.clients.{client_id}");
    if !policy.public {
        if !policy.session_token_audiences.is_empty() {
            return Err(format!(
                "{prefix}.session_token_audiences requires `public: true`"
            ));
        }
        return Ok(());
    }
    if !policy.on_behalf_of_audiences.is_empty() || !policy.requested_claims.is_empty() {
        return Err(format!(
            "{prefix} is public: it may use only the session-token exchange, not \
             on_behalf_of_audiences or requested_claims"
        ));
    }
    if policy.session_token_audiences.is_empty() {
        return Err(format!(
            "{prefix} is public but lists no session_token_audiences"
        ));
    }
    if policy
        .session_token_audiences
        .iter()
        .any(|a| a.trim().is_empty())
    {
        return Err(format!(
            "{prefix}.session_token_audiences has an empty audience"
        ));
    }
    if !has_session_provider {
        return Err(format!(
            "{prefix} is public but token_exchange.session_token_provider is not set"
        ));
    }
    Ok(())
}

/// The longest `requested_subject` accepted.
pub const MAX_REQUESTED_SUBJECT_LEN: usize = 255;

/// Validate the on-behalf-of request envelope: grant type, no actor token, no
/// subject token, an access-token `requested_token_type` when present, and a
/// well-formed `requested_subject`. Returns the subject. Pure.
pub fn validate_on_behalf_of_request(req: &TokenExchangeRequest) -> Result<&str, ExchangeError> {
    if req.grant_type != GRANT_TYPE_TOKEN_EXCHANGE {
        return Err(ExchangeError::UnsupportedGrantType(format!(
            "unsupported grant_type: {}",
            req.grant_type
        )));
    }
    if req
        .actor_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return Err(ExchangeError::UnsupportedActorToken);
    }
    if !req.subject_token.trim().is_empty() {
        return Err(ExchangeError::InvalidRequest(
            "requested_subject cannot be combined with subject_token".to_string(),
        ));
    }
    if let Some(t) = req.requested_token_type.as_deref() {
        if t != TOKEN_TYPE_ACCESS_TOKEN {
            return Err(ExchangeError::InvalidRequest(format!(
                "unsupported requested_token_type: {t}"
            )));
        }
    }
    let subject = req.requested_subject.as_deref().unwrap_or("");
    if subject.is_empty()
        || subject.len() > MAX_REQUESTED_SUBJECT_LEN
        || subject.trim() != subject
        || subject.chars().any(char::is_control)
    {
        return Err(ExchangeError::InvalidRequest(
            "requested_subject must be a non-empty identifier without surrounding whitespace \
             or control characters"
                .to_string(),
        ));
    }
    Ok(subject)
}

/// Parse `expires_in`: a positive integer number of seconds, or absent.
pub fn parse_requested_ttl(raw: Option<&str>) -> Result<Option<u64>, ExchangeError> {
    let Some(raw) = raw else { return Ok(None) };
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .map(Some)
        .ok_or_else(|| {
            ExchangeError::InvalidRequest(
                "expires_in must be a positive integer number of seconds".to_string(),
            )
        })
}

/// Authenticate presented client credentials. No credentials → `Ok(None)`.
/// Credentials with no verifier, an unknown client or a wrong secret →
/// `InvalidClient` (fail closed, never ignored).
async fn authenticate_client(
    req: &TokenExchangeRequest,
    policy: &ExchangePolicy,
) -> Result<Option<VerifiedClient>, ExchangeError> {
    let Some(creds) = &req.client_credentials else {
        return Ok(None);
    };
    let Some(verifier) = &policy.client_verifier else {
        return Err(ExchangeError::InvalidClient);
    };
    // A public client has no secret; a secret presented for one is refused even
    // if an `oauth_clients` row with that id exists.
    if policy.is_public_client(&creds.client_id) {
        return Err(ExchangeError::InvalidClient);
    }
    match verifier
        .verify(&creds.client_id, &creds.client_secret)
        .await
    {
        Ok(Some(client)) => Ok(Some(client)),
        Ok(None) => Err(ExchangeError::InvalidClient),
        Err(e) => Err(ExchangeError::MintFailed(format!("client store: {e}"))),
    }
}

/// Collect the requested claims from the extra form fields. A field is a
/// requested claim only if some configured client may request that name; other
/// unknown fields are ignored as before. A requested claim from a caller that
/// is not authenticated, or whose policy does not list it, is refused.
pub fn collect_requested_claims(
    req: &TokenExchangeRequest,
    policy: &ExchangePolicy,
    client: Option<&VerifiedClient>,
) -> Result<Map<String, Value>, ExchangeError> {
    let allowed: &[String] = client
        .and_then(|c| policy.clients.get(&c.client_id))
        .map(|p| p.requested_claims.as_slice())
        .unwrap_or(&[]);
    let mut out = Map::new();
    for (name, raw) in &req.extra_fields {
        if !policy.is_requestable_claim(name) {
            continue;
        }
        if client.is_none() {
            return Err(ExchangeError::InvalidClient);
        }
        if !allowed.iter().any(|a| a == name) {
            return Err(ExchangeError::UnauthorizedClient(format!(
                "this client may not request the {name:?} claim"
            )));
        }
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()));
        out.insert(name.clone(), value);
    }
    Ok(out)
}

/// A claim of the verified subject: a profile trait, else another claim.
fn subject_claim<'a>(subject: &'a Identity, name: &str) -> Option<&'a Value> {
    subject
        .traits
        .get(name)
        .or_else(|| subject.metadata_public.get(name))
}

/// Add per-audience and requested claims to the delegated claims. Precedence,
/// lowest first: claims copied from the subject, requested claims, fixed
/// audience claims. Reserved claims are never written. Pure.
pub fn apply_exchange_claims(
    additional: &mut Value,
    subject: &Identity,
    audience_policy: Option<&ExchangeAudiencePolicy>,
    requested: &Map<String, Value>,
) {
    let Value::Object(map) = additional else {
        return;
    };
    let mut put = |name: &str, value: &Value| {
        if !is_exchange_reserved_claim(name) {
            map.insert(name.to_string(), value.clone());
        }
    };
    if let Some(policy) = audience_policy {
        for name in &policy.copy_claims {
            if let Some(value) = subject_claim(subject, name) {
                put(name, value);
            }
        }
    }
    for (name, value) in requested {
        put(name, value);
    }
    if let Some(policy) = audience_policy {
        for (name, value) in &policy.claims {
            put(name, value);
        }
    }
}

/// The subject, granted scopes and audience of an on-behalf-of exchange. The
/// client must be authenticated and allow-listed for the requested audience;
/// scopes are capped by the client's own grant.
fn on_behalf_of_grant<'r>(
    req: &'r TokenExchangeRequest,
    subject: &str,
    client: Option<&VerifiedClient>,
    policy: &ExchangePolicy,
) -> Result<(Identity, Vec<String>, &'r str), ExchangeError> {
    let client = client.ok_or(ExchangeError::InvalidClient)?;
    let permitted = policy
        .clients
        .get(&client.client_id)
        .map(|p| p.on_behalf_of_audiences.as_slice())
        .unwrap_or(&[]);
    if permitted.is_empty() {
        return Err(ExchangeError::UnauthorizedClient(
            "this client may not request tokens on behalf of a person".to_string(),
        ));
    }
    let audience = req
        .resource
        .as_deref()
        .or(req.audience.as_deref())
        .filter(|a| !a.trim().is_empty())
        .ok_or_else(|| {
            ExchangeError::InvalidRequest(
                "an on-behalf-of exchange requires `audience` or `resource`".to_string(),
            )
        })?;
    if !permitted.iter().any(|a| a == audience) {
        return Err(ExchangeError::InvalidTarget(format!(
            "this client may not request on-behalf-of tokens for audience {audience:?}"
        )));
    }
    let granted = downscope(req.scope.as_deref(), &client.scopes)?;
    let identity = Identity {
        id: subject.to_string(),
        ..Default::default()
    };
    Ok((identity, granted, audience))
}

/// End-to-end token exchange with the empty [`ExchangePolicy`]: the plain
/// RFC 8693 exchange. See [`exchange_with_policy`].
pub async fn exchange(
    req: &TokenExchangeRequest,
    verifier: &Arc<dyn Authenticator>,
    minter: &SharedJwtMinter,
    delegated_ttl_seconds: Option<u64>,
    delegate: Option<&HydraDelegate>,
) -> Result<Value, ExchangeError> {
    exchange_with_policy(
        req,
        verifier,
        minter,
        delegated_ttl_seconds,
        delegate,
        &ExchangePolicy::default(),
    )
    .await
}

/// End-to-end token exchange: validate → (delegate to Hydra when configured, OR
/// authenticate the client → resolve the subject (verify `subject_token`, or
/// the on-behalf-of `requested_subject`) → downscope → apply audience and
/// requested claims → mint a delegated token via [`JwtMinter`]).
/// Returns the RFC 8693 response body or a mapped [`ExchangeError`].
///
/// Fail-closed at every step: a bad grant, an `actor_token`, an unverifiable
/// subject token, bad client credentials, a capability the client is not
/// allow-listed for, scope escalation, an unavailable minter, or a Hydra
/// delegate error all reject rather than issue a token.
///
/// [`JwtMinter`]: crate::auth::jwt_mint::JwtMinter
pub async fn exchange_with_policy(
    req: &TokenExchangeRequest,
    verifier: &Arc<dyn Authenticator>,
    minter: &SharedJwtMinter,
    delegated_ttl_seconds: Option<u64>,
    delegate: Option<&HydraDelegate>,
    policy: &ExchangePolicy,
) -> Result<Value, ExchangeError> {
    // A Kratos session-token type, or a public client, selects the
    // session-token exchange. Checked first: a session token is never relayed
    // to a Hydra delegate, and a public client can do nothing else.
    if req.subject_token_type.as_deref() == Some(TOKEN_TYPE_KRATOS_SESSION_TOKEN)
        || req
            .public_client_id
            .as_deref()
            .is_some_and(|id| policy.is_public_client(id))
    {
        return exchange_session_token(req, minter, delegated_ttl_seconds, policy).await;
    }
    // A `requested_subject` (even an empty one) selects the on-behalf-of path,
    // whose envelope has no subject token.
    let on_behalf_of = req.requested_subject.is_some();
    let obo_subject = if on_behalf_of {
        Some(validate_on_behalf_of_request(req)?)
    } else {
        // In delegate mode, a rejected actor_token is a delegate-path denial — record
        // it before returning so the metric captures every delegate outcome.
        if let Err(e) = validate_request(req) {
            if delegate.is_some() && matches!(e, ExchangeError::UnsupportedActorToken) {
                crate::metrics::record_delegate("deny_actor_token");
            }
            return Err(e);
        }
        None
    };

    // Federate-first: when a Hydra delegate is configured, proxy the exchange to
    // it (Hydra owns RFC 8693) instead of minting locally. The local-only
    // capabilities are refused rather than silently dropped.
    if let Some(delegate) = delegate {
        if on_behalf_of || req.expires_in.is_some() || req.client_credentials.is_some() {
            crate::metrics::record_delegate("deny_unsupported");
            return Err(ExchangeError::InvalidRequest(
                "requested_subject, expires_in and client authentication are not supported \
                 when the exchange is delegated to Hydra"
                    .to_string(),
            ));
        }
        return delegate_exchange_to_hydra(delegate, req).await;
    }

    // Local-mint path. Instrument every outcome (result label) so this mode is
    // observable symmetrically with the Hydra-delegate path. The `?`-propagation
    // is unrolled into explicit arms: each fail-closed exit records its reason.
    let deny = |reason: &'static str, e: ExchangeError| {
        crate::metrics::record_local_exchange(reason);
        e
    };
    let requested_ttl =
        parse_requested_ttl(req.expires_in.as_deref()).map_err(|e| deny("deny_request", e))?;
    let client = authenticate_client(req, policy)
        .await
        .map_err(|e| deny("deny_client", e))?;
    let requested_claims = collect_requested_claims(req, policy, client.as_ref())
        .map_err(|e| deny("deny_client", e))?;

    let (identity, granted, audience, actor) = if let Some(subject) = obo_subject {
        let (identity, granted, audience) =
            on_behalf_of_grant(req, subject, client.as_ref(), policy).map_err(|e| {
                let reason = match e {
                    ExchangeError::InvalidScope(_) => "deny_downscope",
                    ExchangeError::InvalidRequest(_) => "deny_request",
                    _ => "deny_client",
                };
                deny(reason, e)
            })?;
        (identity, granted, Some(audience), client.as_ref())
    } else {
        let subject = verify_subject_token(verifier, &req.subject_token)
            .await
            .map_err(|e| deny("deny_verify", e))?;
        if let Some(public_actor) = public_client_actor(&subject.identity, policy) {
            tracing::warn!(
                target: "flint_gate::audit",
                event = "token_exchange.public_subject_refused",
                act_sub = public_actor,
                sub = %subject.identity.id,
                client_id = client.as_ref().map(|c| c.client_id.as_str()).unwrap_or_default(),
                "refused a subject token minted for a public client"
            );
            return Err(deny(
                "deny_verify",
                ExchangeError::InvalidSubjectToken(
                    "a token issued to a public client cannot be exchanged".to_string(),
                ),
            ));
        }
        let granted = downscope(req.scope.as_deref(), &subject.scopes)
            .map_err(|e| deny("deny_downscope", e))?;
        // RFC 8707: prefer `resource`, then `audience`, as the issued token's `aud`.
        let audience = req.resource.as_deref().or(req.audience.as_deref());
        (subject.identity, granted, audience, client.as_ref())
    };

    // `act.sub` is the authenticated client when there is one; otherwise the
    // subject itself, so the token is still explicitly a delegated token.
    let actor_sub = actor.map(|c| c.client_id.as_str());
    let mut additional = build_delegated_claims(&identity, &granted, audience, actor_sub);
    apply_exchange_claims(
        &mut additional,
        &identity,
        audience.and_then(|a| policy.audiences.get(a)),
        &requested_claims,
    );

    let guard = minter.read().await;
    let Some(m) = guard.as_ref() else {
        return Err(deny(
            "mint_failed",
            ExchangeError::MintFailed("JWT minter is not configured".to_string()),
        ));
    };
    let base_ttl = delegated_ttl_seconds.unwrap_or_else(|| m.default_ttl_seconds());
    let ttl = requested_ttl.map_or(base_ttl, |r| r.min(base_ttl));
    let token = m
        .mint(&identity, Some(&additional), Some(ttl))
        .map_err(|e| deny("mint_failed", ExchangeError::MintFailed(e.to_string())))?;

    if on_behalf_of {
        tracing::info!(
            target: "flint_gate::audit",
            event = "token_exchange.on_behalf_of",
            client_id = actor_sub.unwrap_or_default(),
            sub = %identity.id,
            aud = audience.unwrap_or_default(),
            ttl_seconds = ttl,
            "issued an on-behalf-of token"
        );
    }
    crate::metrics::record_local_exchange("success");
    Ok(token_exchange_response(token, &granted, ttl))
}

/// The `act.sub` of a verified subject token when it names a configured public
/// client: the token came from the session-token exchange and is terminal. It
/// may not be exchanged again, which would re-issue it with a confidential
/// client or the subject itself as the actor and erase the public origin. Pure.
fn public_client_actor<'a>(subject: &'a Identity, policy: &ExchangePolicy) -> Option<&'a str> {
    subject_claim(subject, "act")
        .and_then(|act| act.get("sub"))
        .and_then(Value::as_str)
        .filter(|sub| policy.is_public_client(sub))
}

/// Validate a session-token exchange and resolve its public client and
/// audience. The caller must be a configured public client, send the Kratos
/// session-token type, and ask for one of its `session_token_audiences`. Pure.
pub fn validate_session_token_request<'r>(
    req: &'r TokenExchangeRequest,
    policy: &ExchangePolicy,
) -> Result<(&'r str, &'r str), ExchangeError> {
    if req.grant_type != GRANT_TYPE_TOKEN_EXCHANGE {
        return Err(ExchangeError::UnsupportedGrantType(format!(
            "unsupported grant_type: {}",
            req.grant_type
        )));
    }
    // Only a registered public client, identified by `client_id` alone, may
    // exchange a session token. Client credentials, an unknown client_id or a
    // confidential client's id are refused.
    let client_id = req
        .public_client_id
        .as_deref()
        .filter(|id| policy.is_public_client(id))
        .ok_or(ExchangeError::InvalidClient)?;
    if req.subject_token_type.as_deref() != Some(TOKEN_TYPE_KRATOS_SESSION_TOKEN) {
        return Err(ExchangeError::UnauthorizedClient(format!(
            "a public client may only exchange a Kratos session token \
             (subject_token_type={TOKEN_TYPE_KRATOS_SESSION_TOKEN})"
        )));
    }
    if req.requested_subject.is_some() {
        return Err(ExchangeError::UnauthorizedClient(
            "a public client may not request tokens on behalf of a person".to_string(),
        ));
    }
    if let Some(name) = req
        .extra_fields
        .keys()
        .find(|name| policy.is_requestable_claim(name))
    {
        return Err(ExchangeError::UnauthorizedClient(format!(
            "a public client may not request the {name:?} claim"
        )));
    }
    if req
        .actor_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return Err(ExchangeError::UnsupportedActorToken);
    }
    if let Some(t) = req.requested_token_type.as_deref() {
        if t != TOKEN_TYPE_ACCESS_TOKEN {
            return Err(ExchangeError::InvalidRequest(format!(
                "unsupported requested_token_type: {t}"
            )));
        }
    }
    if req.subject_token.trim().is_empty() {
        return Err(ExchangeError::InvalidRequest(
            "subject_token is required".to_string(),
        ));
    }
    let audience = req
        .resource
        .as_deref()
        .or(req.audience.as_deref())
        .filter(|a| !a.trim().is_empty())
        .ok_or_else(|| {
            ExchangeError::InvalidRequest(
                "a session-token exchange requires `audience` or `resource`".to_string(),
            )
        })?;
    let permitted = &policy.clients[client_id].session_token_audiences;
    if !permitted.iter().any(|a| a == audience) {
        return Err(ExchangeError::InvalidTarget(format!(
            "this client may not request tokens for audience {audience:?}"
        )));
    }
    Ok((client_id, audience))
}

/// The Kratos session-token exchange for a public client: validate → verify
/// the session with Kratos → mint a token for the allowed audience with
/// `sub` = the Kratos identity (traits merged as for a browser session) and
/// `act.sub` = the public client. The lifetime is the exchange TTL, shortened
/// by `expires_in` and never past the Kratos session's own expiry. A Kratos
/// rejection or outage is `401 invalid_grant`; no token is issued.
async fn exchange_session_token(
    req: &TokenExchangeRequest,
    minter: &SharedJwtMinter,
    delegated_ttl_seconds: Option<u64>,
    policy: &ExchangePolicy,
) -> Result<Value, ExchangeError> {
    let deny = |reason: &'static str, e: ExchangeError| {
        crate::metrics::record_local_exchange(reason);
        e
    };
    let (client_id, audience) = validate_session_token_request(req, policy).map_err(|e| {
        let reason = match e {
            ExchangeError::InvalidRequest(_) | ExchangeError::UnsupportedActorToken => {
                "deny_request"
            }
            _ => "deny_client",
        };
        deny(reason, e)
    })?;
    let requested_ttl =
        parse_requested_ttl(req.expires_in.as_deref()).map_err(|e| deny("deny_request", e))?;
    // A Kratos session carries no OAuth scopes: any requested scope is refused.
    let granted = downscope(req.scope.as_deref(), &[]).map_err(|e| deny("deny_downscope", e))?;
    let Some(verifier) = &policy.session_verifier else {
        return Err(deny(
            "mint_failed",
            ExchangeError::MintFailed("no Kratos session-token verifier is configured".into()),
        ));
    };
    let identity = verifier
        .verify_session_token(&req.subject_token)
        .await
        .map_err(|e| {
            tracing::warn!(client_id, error = %e, "session-token exchange: Kratos refused or unreachable");
            deny(
                "deny_verify",
                ExchangeError::InvalidGrant("invalid, expired or unverifiable session".into()),
            )
        })?;

    let mut additional =
        build_delegated_claims(&identity, &granted, Some(audience), Some(client_id));
    apply_exchange_claims(
        &mut additional,
        &identity,
        policy.audiences.get(audience),
        &Map::new(),
    );

    let guard = minter.read().await;
    let Some(m) = guard.as_ref() else {
        return Err(deny(
            "mint_failed",
            ExchangeError::MintFailed("JWT minter is not configured".to_string()),
        ));
    };
    let base_ttl = delegated_ttl_seconds.unwrap_or_else(|| m.default_ttl_seconds());
    let mut ttl = requested_ttl.map_or(base_ttl, |r| r.min(base_ttl));
    if let Some(expires_at) = identity.session_expires_at {
        let remaining = (expires_at - chrono::Utc::now()).num_seconds();
        ttl = ttl.min(u64::try_from(remaining).unwrap_or(0));
    }
    if ttl == 0 {
        return Err(deny(
            "deny_verify",
            ExchangeError::InvalidGrant("the Kratos session is about to expire".into()),
        ));
    }
    let token = m
        .mint(&identity, Some(&additional), Some(ttl))
        .map_err(|e| deny("mint_failed", ExchangeError::MintFailed(e.to_string())))?;

    tracing::info!(
        target: "flint_gate::audit",
        event = "token_exchange.session_token",
        client_id,
        sub = %identity.id,
        aud = audience,
        ttl_seconds = ttl,
        "issued a token for a Kratos session"
    );
    crate::metrics::record_local_exchange("success");
    Ok(token_exchange_response(token, &granted, ttl))
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
            ..Default::default()
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
            issuer: Some("https://identity.example.test".into()),
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
        assert_eq!(
            scopes_from_identity(&identity),
            vec!["read", "write", "admin"]
        );
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
        let resp = token_exchange_response("tok.abc".into(), &["read".into()], 60);
        assert_eq!(resp["access_token"], "tok.abc");
        assert_eq!(resp["expires_in"], 60);
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
            ..Default::default()
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
            ..Default::default()
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
        assert!(exchange(&exchange_req(None), &verifier, &m, None, None)
            .await
            .is_ok());
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

    // ── Local-exchange outcome metering ───────────────────────────────────
    // The recorder is process-global, so these assert PRESENCE of each result
    // label after driving the outcome (robust under parallel execution). The
    // deny_verify / deny_downscope / mint_failed labels are UNIQUE to the local
    // path (the delegate path uses deny_transport / deny_non2xx / deny_badjson),
    // so their presence unambiguously proves this branch recorded them.

    #[tokio::test]
    async fn local_exchange_success_is_metered() {
        crate::metrics::install_recorder();
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read write",
        });
        let m = minter().await;
        exchange(&exchange_req(Some("read")), &verifier, &m, None, None)
            .await
            .expect("valid exchange");
        let out = crate::metrics::render();
        assert!(
            out.contains("flint_local_exchange_total"),
            "local success not metered:\n{out}"
        );
    }

    #[tokio::test]
    async fn local_exchange_deny_verify_is_metered() {
        crate::metrics::install_recorder();
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: false,
            scopes: "read",
        });
        let m = minter().await;
        let err = exchange(&exchange_req(None), &verifier, &m, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidSubjectToken(_)));
        let out = crate::metrics::render();
        assert!(
            out.contains("result=\"deny_verify\""),
            "deny_verify not metered:\n{out}"
        );
    }

    #[tokio::test]
    async fn local_exchange_deny_downscope_is_metered() {
        crate::metrics::install_recorder();
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        let m = minter().await;
        // Requesting a scope the subject lacks → downscope rejects (escalation).
        let err = exchange(&exchange_req(Some("read admin")), &verifier, &m, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_scope");
        let out = crate::metrics::render();
        assert!(
            out.contains("result=\"deny_downscope\""),
            "deny_downscope not metered:\n{out}"
        );
    }

    #[tokio::test]
    async fn local_exchange_mint_failed_is_metered_when_minter_absent() {
        crate::metrics::install_recorder();
        let verifier: Arc<dyn Authenticator> = Arc::new(StubVerifier {
            accept: true,
            scopes: "read",
        });
        // A minter guard holding None → mint step fails closed (MintFailed).
        let m: SharedJwtMinter = Arc::new(RwLock::new(None));
        let err = exchange(&exchange_req(Some("read")), &verifier, &m, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::MintFailed(_)));
        let out = crate::metrics::render();
        assert!(
            out.contains("result=\"mint_failed\""),
            "mint_failed not metered:\n{out}"
        );
    }

    // ── Hydra-delegate exchange ───────────────────────────────────────────

    #[tokio::test]
    async fn delegate_forwards_to_hydra_and_returns_its_token() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth2/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "hydra-minted-token",
                "token_type": "Bearer",
                "scope": "read"
            })))
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

#[cfg(test)]
mod policy_tests {
    //! Client authentication, on-behalf-of, requested claims, per-audience
    //! claims and requested lifetime on the gateway-local exchange.
    use super::*;
    use crate::auth::jwt_mint::JwtMinter;
    use crate::auth::{AuthMethod, AuthResult};
    use crate::config::types::JwtConfig;
    use base64::Engine;
    use http::request::Parts;
    use tokio::sync::RwLock;

    /// A subject token for `person-1` with profile, tenant and app-role claims.
    struct PersonVerifier;

    #[async_trait::async_trait]
    impl Authenticator for PersonVerifier {
        async fn authenticate(&self, _parts: &Parts) -> Result<AuthResult, AuthError> {
            Ok(AuthResult {
                identity: Identity {
                    id: "person-1".into(),
                    traits: json!({ "email": "p@example.test" }),
                    metadata_public: json!({
                        "scope": "chat",
                        "tenant_id": "tenant-a",
                        "role": "admin",
                        "aud": "knowduet-gateway",
                    }),
                    ..Default::default()
                },
                method: AuthMethod::BearerJwt,
            })
        }
    }

    /// The client store: `openbot-server` and `knowduet-gateway`, secret `pw`.
    struct StubClients;

    #[async_trait::async_trait]
    impl ClientVerifier for StubClients {
        async fn verify(
            &self,
            client_id: &str,
            client_secret: &str,
        ) -> anyhow::Result<Option<VerifiedClient>> {
            let known = ["openbot-server", "knowduet-gateway", "other-service"];
            Ok(
                (known.contains(&client_id) && client_secret == "pw").then(|| VerifiedClient {
                    client_id: client_id.to_string(),
                    scopes: vec!["chat".into(), "runs".into()],
                }),
            )
        }
    }

    fn policy() -> ExchangePolicy {
        let mut p = ExchangePolicy {
            client_verifier: Some(Arc::new(StubClients)),
            ..Default::default()
        };
        p.clients.insert(
            "openbot-server".into(),
            ExchangeClientPolicy {
                on_behalf_of_audiences: vec!["knowduet-gateway".into()],
                requested_claims: vec![],
                ..Default::default()
            },
        );
        p.clients.insert(
            "knowduet-gateway".into(),
            ExchangeClientPolicy {
                on_behalf_of_audiences: vec![],
                requested_claims: vec!["topics".into()],
                ..Default::default()
            },
        );
        p.audiences.insert(
            "flint-forge".into(),
            ExchangeAudiencePolicy {
                copy_claims: vec!["tenant_id".into(), "email".into()],
                claims: json!({ "role": "authenticated" })
                    .as_object()
                    .unwrap()
                    .clone(),
            },
        );
        p
    }

    async fn minter() -> SharedJwtMinter {
        let cfg = JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("test-secret-key-minimum-length".into()),
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
            ..Default::default()
        };
        Arc::new(RwLock::new(Some(
            JwtMinter::from_config(&cfg).await.unwrap(),
        )))
    }

    fn verifier() -> Arc<dyn Authenticator> {
        Arc::new(PersonVerifier)
    }

    fn creds(client_id: &str, secret: &str) -> Option<ClientCredentials> {
        Some(ClientCredentials {
            client_id: client_id.into(),
            client_secret: secret.into(),
        })
    }

    fn obo_req(subject: &str, audience: &str) -> TokenExchangeRequest {
        TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            requested_subject: Some(subject.into()),
            audience: Some(audience.into()),
            client_credentials: creds("openbot-server", "pw"),
            ..Default::default()
        }
    }

    fn ticket_req() -> TokenExchangeRequest {
        let mut extra = BTreeMap::new();
        extra.insert(
            "topics".to_string(),
            r#"["user:person-1","bot:b1"]"#.to_string(),
        );
        TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "gateway.subject.jwt".into(),
            subject_token_type: Some(TOKEN_TYPE_ACCESS_TOKEN.into()),
            audience: Some("fabric".into()),
            expires_in: Some("60".into()),
            client_credentials: creds("knowduet-gateway", "pw"),
            extra_fields: extra,
            ..Default::default()
        }
    }

    async fn run(req: &TokenExchangeRequest, p: &ExchangePolicy) -> Result<Value, ExchangeError> {
        exchange_with_policy(req, &verifier(), &minter().await, None, None, p).await
    }

    /// The minted token's claims (signature already covered by the minter).
    fn claims(resp: &Value) -> Value {
        let token = resp["access_token"].as_str().unwrap();
        let payload = token.split('.').nth(1).unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ── On-behalf-of (FG-7) ────────────────────────────────────────────────

    #[tokio::test]
    async fn on_behalf_of_names_the_person_and_records_the_client_as_actor() {
        let resp = run(&obo_req("user-9", "knowduet-gateway"), &policy())
            .await
            .expect("allow-listed on-behalf-of exchange");
        let c = claims(&resp);
        assert_eq!(c["sub"], "user-9");
        assert_eq!(c["aud"], "knowduet-gateway");
        assert_eq!(c["act"]["sub"], "openbot-server");
        assert_eq!(c["flint_kind"], "agent");
        assert!(c["jti"].as_str().is_some_and(|j| !j.is_empty()));
        assert_eq!(c["exp"].as_i64().unwrap() - c["iat"].as_i64().unwrap(), 300);
        assert_eq!(resp["expires_in"], 300);
        assert_eq!(resp["issued_token_type"], TOKEN_TYPE_ACCESS_TOKEN);
        // Scopes are the client's grant, not anyone's subject token.
        assert_eq!(c["scope"], "chat runs");
    }

    #[tokio::test]
    async fn on_behalf_of_requires_client_authentication() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.client_credentials = None;
        assert_eq!(
            run(&r, &policy()).await.unwrap_err(),
            ExchangeError::InvalidClient
        );
    }

    #[tokio::test]
    async fn on_behalf_of_refuses_a_wrong_secret() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.client_credentials = creds("openbot-server", "wrong");
        let err = run(&r, &policy()).await.unwrap_err();
        assert_eq!(err, ExchangeError::InvalidClient);
        assert_eq!(err.http_status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn on_behalf_of_refuses_a_client_that_is_not_allow_listed() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.client_credentials = creds("other-service", "pw");
        let err = run(&r, &policy()).await.unwrap_err();
        assert!(matches!(err, ExchangeError::UnauthorizedClient(_)));
        // A client allow-listed only for requested claims cannot impersonate.
        r.client_credentials = creds("knowduet-gateway", "pw");
        let err = run(&r, &policy()).await.unwrap_err();
        assert!(matches!(err, ExchangeError::UnauthorizedClient(_)));
    }

    #[tokio::test]
    async fn on_behalf_of_refuses_an_audience_outside_the_allow_list() {
        let err = run(&obo_req("user-9", "flint-forge"), &policy())
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::InvalidTarget(_)));
        assert_eq!(err.oauth_code(), "invalid_target");
    }

    #[tokio::test]
    async fn on_behalf_of_requires_an_audience() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.audience = None;
        assert!(matches!(
            run(&r, &policy()).await.unwrap_err(),
            ExchangeError::InvalidRequest(_)
        ));
    }

    #[tokio::test]
    async fn on_behalf_of_refuses_malformed_envelopes() {
        let mut with_subject_token = obo_req("user-9", "knowduet-gateway");
        with_subject_token.subject_token = "a.b.c".into();
        let mut wrong_type = obo_req("user-9", "knowduet-gateway");
        wrong_type.requested_token_type = Some("urn:ietf:params:oauth:token-type:id_token".into());
        for bad in [
            with_subject_token,
            wrong_type,
            obo_req("", "knowduet-gateway"),
            obo_req(" user-9", "knowduet-gateway"),
            obo_req("user\n9", "knowduet-gateway"),
            obo_req(
                &"x".repeat(MAX_REQUESTED_SUBJECT_LEN + 1),
                "knowduet-gateway",
            ),
        ] {
            assert!(matches!(
                run(&bad, &policy()).await.unwrap_err(),
                ExchangeError::InvalidRequest(_)
            ));
        }
        let mut with_actor = obo_req("user-9", "knowduet-gateway");
        with_actor.actor_token = Some("x.y.z".into());
        assert_eq!(
            run(&with_actor, &policy()).await.unwrap_err(),
            ExchangeError::UnsupportedActorToken
        );
    }

    #[tokio::test]
    async fn on_behalf_of_accepts_the_access_token_type() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.requested_token_type = Some(TOKEN_TYPE_ACCESS_TOKEN.into());
        assert!(run(&r, &policy()).await.is_ok());
    }

    #[tokio::test]
    async fn on_behalf_of_scope_is_capped_by_the_client_grant() {
        let mut r = obo_req("user-9", "knowduet-gateway");
        r.scope = Some("chat admin".into());
        assert!(matches!(
            run(&r, &policy()).await.unwrap_err(),
            ExchangeError::InvalidScope(_)
        ));
    }

    #[tokio::test]
    async fn on_behalf_of_is_refused_by_the_default_policy() {
        // `exchange()` (no policy) has no client verifier → fail closed.
        let err = exchange(
            &obo_req("user-9", "knowduet-gateway"),
            &verifier(),
            &minter().await,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExchangeError::InvalidClient);
    }

    // ── Requested claims and lifetime (FG-9) ───────────────────────────────

    #[tokio::test]
    async fn realtime_ticket_carries_topics_audience_short_ttl_and_fresh_jti() {
        let p = policy();
        let first = run(&ticket_req(), &p).await.expect("ticket exchange");
        let second = run(&ticket_req(), &p).await.expect("ticket exchange");
        let c = claims(&first);
        assert_eq!(c["sub"], "person-1");
        assert_eq!(c["aud"], "fabric");
        assert_eq!(c["topics"], json!(["user:person-1", "bot:b1"]));
        assert_eq!(c["act"]["sub"], "knowduet-gateway");
        assert_eq!(c["exp"].as_i64().unwrap() - c["iat"].as_i64().unwrap(), 60);
        assert_eq!(first["expires_in"], 60);
        let jti = c["jti"].as_str().unwrap().to_string();
        assert!(!jti.is_empty());
        assert_ne!(claims(&second)["jti"].as_str().unwrap(), jti);
    }

    #[tokio::test]
    async fn requested_claim_is_refused_from_an_unauthenticated_caller() {
        let mut r = ticket_req();
        r.client_credentials = None;
        assert_eq!(
            run(&r, &policy()).await.unwrap_err(),
            ExchangeError::InvalidClient
        );
    }

    #[tokio::test]
    async fn requested_claim_is_refused_from_a_client_not_allowed_it() {
        let mut r = ticket_req();
        r.client_credentials = creds("openbot-server", "pw");
        assert!(matches!(
            run(&r, &policy()).await.unwrap_err(),
            ExchangeError::UnauthorizedClient(_)
        ));
    }

    #[tokio::test]
    async fn unknown_extra_fields_are_still_ignored() {
        let mut r = ticket_req();
        r.extra_fields.clear();
        r.extra_fields.insert("unrelated".into(), "x".into());
        r.client_credentials = None;
        let c = claims(&run(&r, &policy()).await.unwrap());
        assert!(c.get("unrelated").is_none());
        // Without an authenticated client the subject is its own actor.
        assert_eq!(c["act"]["sub"], "person-1");
    }

    #[tokio::test]
    async fn non_json_claim_value_is_copied_as_a_string() {
        let mut r = ticket_req();
        r.extra_fields
            .insert("topics".into(), "user:person-1".into());
        assert_eq!(
            claims(&run(&r, &policy()).await.unwrap())["topics"],
            "user:person-1"
        );
    }

    #[tokio::test]
    async fn expires_in_only_shortens_the_lifetime() {
        let mut r = ticket_req();
        r.expires_in = Some("86400".into());
        let resp = run(&r, &policy()).await.unwrap();
        let c = claims(&resp);
        assert_eq!(c["exp"].as_i64().unwrap() - c["iat"].as_i64().unwrap(), 300);
        assert_eq!(resp["expires_in"], 300);
    }

    #[tokio::test]
    async fn expires_in_must_be_a_positive_integer() {
        for bad in ["0", "-5", "abc", ""] {
            let mut r = ticket_req();
            r.expires_in = Some(bad.into());
            assert!(matches!(
                run(&r, &policy()).await.unwrap_err(),
                ExchangeError::InvalidRequest(_)
            ));
        }
    }

    #[tokio::test]
    async fn client_credentials_without_a_store_are_refused() {
        let mut p = policy();
        p.client_verifier = None;
        assert_eq!(
            run(&ticket_req(), &p).await.unwrap_err(),
            ExchangeError::InvalidClient
        );
    }

    #[test]
    fn requested_claims_never_override_reserved_claims() {
        let subject = Identity {
            id: "person-1".into(),
            ..Default::default()
        };
        let mut additional =
            build_delegated_claims(&subject, &["chat".into()], Some("fabric"), Some("gw"));
        let requested = json!({ "sub": "root", "aud": "x", "jti": "j", "act": {}, "topics": [] });
        apply_exchange_claims(
            &mut additional,
            &subject,
            None,
            requested.as_object().unwrap(),
        );
        assert_eq!(additional["aud"], "fabric");
        assert_eq!(additional["act"]["sub"], "gw");
        assert!(additional.get("sub").is_none());
        assert!(additional.get("jti").is_none());
        assert_eq!(additional["topics"], json!([]));
    }

    // ── Per-audience claims (FG-8) ─────────────────────────────────────────

    #[tokio::test]
    async fn audience_policy_copies_tenant_and_fixes_role() {
        let r = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "gateway.subject.jwt".into(),
            audience: Some("flint-forge".into()),
            ..Default::default()
        };
        let c = claims(&run(&r, &policy()).await.unwrap());
        assert_eq!(c["aud"], "flint-forge");
        assert_eq!(c["tenant_id"], "tenant-a");
        // The subject's app role is not passed through; forge gets its PG role.
        assert_eq!(c["role"], "authenticated");
        assert_eq!(c["email"], "p@example.test");
    }

    #[tokio::test]
    async fn audience_policy_applies_only_to_its_audience() {
        let r = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: "gateway.subject.jwt".into(),
            audience: Some("uar".into()),
            ..Default::default()
        };
        let c = claims(&run(&r, &policy()).await.unwrap());
        assert!(c.get("tenant_id").is_none());
        assert!(c.get("role").is_none());
    }

    #[tokio::test]
    async fn audience_policy_fixed_claims_apply_to_on_behalf_of_tokens() {
        let mut p = policy();
        p.clients
            .get_mut("openbot-server")
            .unwrap()
            .on_behalf_of_audiences
            .push("flint-forge".into());
        let c = claims(&run(&obo_req("user-9", "flint-forge"), &p).await.unwrap());
        assert_eq!(c["sub"], "user-9");
        assert_eq!(c["role"], "authenticated");
        // Nothing to copy: an on-behalf-of subject carries no claims.
        assert!(c.get("tenant_id").is_none());
    }

    // ── Delegate mode and startup validation ──────────────────────────────

    #[tokio::test]
    async fn delegate_mode_refuses_local_only_capabilities() {
        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: "http://127.0.0.1:1/oauth2/token".into(),
        };
        for r in [obo_req("user-9", "knowduet-gateway"), ticket_req()] {
            let err = exchange_with_policy(
                &r,
                &verifier(),
                &minter().await,
                None,
                Some(&delegate),
                &policy(),
            )
            .await
            .unwrap_err();
            assert!(matches!(err, ExchangeError::InvalidRequest(_)));
        }
    }

    fn config_with(
        clients: &[(&str, ExchangeClientPolicy)],
        audiences: &[(&str, ExchangeAudiencePolicy)],
    ) -> TokenExchangeConfig {
        TokenExchangeConfig {
            clients: clients
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            audiences: audiences
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn policy_validation_accepts_the_knowduet_shape() {
        let p = policy();
        let cfg = config_with(
            &[
                ("openbot-server", p.clients["openbot-server"].clone()),
                ("knowduet-gateway", p.clients["knowduet-gateway"].clone()),
            ],
            &[("flint-forge", p.audiences["flint-forge"].clone())],
        );
        assert!(validate_exchange_policy(&cfg).is_ok());
    }

    #[test]
    fn policy_validation_refuses_reserved_and_grant_parameter_names() {
        for name in [
            "sub",
            "aud",
            "exp",
            "jti",
            "act",
            "scope",
            "flint_kind",
            "client_secret",
            "expires_in",
            "",
        ] {
            let cfg = config_with(
                &[(
                    "c",
                    ExchangeClientPolicy {
                        requested_claims: vec![name.into()],
                        ..Default::default()
                    },
                )],
                &[],
            );
            assert!(validate_exchange_policy(&cfg).is_err(), "{name:?}");
        }
        for name in ["iss", "sub", "act"] {
            let cfg = config_with(
                &[],
                &[(
                    "forge",
                    ExchangeAudiencePolicy {
                        claims: json!({ name: "x" }).as_object().unwrap().clone(),
                        ..Default::default()
                    },
                )],
            );
            assert!(validate_exchange_policy(&cfg).is_err(), "{name:?}");
            let cfg = config_with(
                &[],
                &[(
                    "forge",
                    ExchangeAudiencePolicy {
                        copy_claims: vec![name.into()],
                        ..Default::default()
                    },
                )],
            );
            assert!(validate_exchange_policy(&cfg).is_err(), "{name:?}");
        }
    }

    #[test]
    fn policy_validation_refuses_policy_with_hydra_delegate() {
        let mut cfg = config_with(&[("c", ExchangeClientPolicy::default())], &[]);
        cfg.delegate_to_hydra = true;
        assert!(validate_exchange_policy(&cfg).is_err());
    }

    #[test]
    fn client_credentials_debug_hides_the_secret() {
        let c = ClientCredentials {
            client_id: "svc".into(),
            client_secret: "s3cret".into(),
        };
        let shown = format!("{c:?}");
        assert!(shown.contains("svc"));
        assert!(!shown.contains("s3cret"));
    }
}

#[cfg(test)]
mod session_token_tests {
    //! Kratos session-token exchange for public clients, against a stub Kratos.
    use super::*;
    use crate::auth::jwt_mint::JwtMinter;
    use crate::auth::kratos::KratosAuthenticator;
    use crate::config::types::{JwtConfig, KratosAuthConfig};
    use base64::Engine;
    use tokio::sync::RwLock;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const DESKTOP: &str = "knowduet-desktop";

    fn kratos(base_url: &str) -> Arc<dyn SessionTokenVerifier> {
        Arc::new(KratosAuthenticator::new(
            KratosAuthConfig {
                base_url: base_url.to_string(),
                issuer: None,
                forward_cookies: true,
                session_cookie: "ory_kratos_session".to_string(),
            },
            reqwest::Client::new(),
        ))
    }

    /// Kratos answering `whoami` for `X-Session-Token: good-token`.
    async fn kratos_with_session(expires_at: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sessions/whoami"))
            .and(header("x-session-token", "good-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "session-1",
                "active": true,
                "expires_at": expires_at,
                "identity": {
                    "id": "identity-7",
                    "traits": { "email": "person@example.test" },
                    "metadata_public": { "tenant_id": "tenant-a", "flint_kind": "service" }
                }
            })))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions/whoami"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        server
    }

    fn policy(session_verifier: Arc<dyn SessionTokenVerifier>) -> ExchangePolicy {
        let mut p = ExchangePolicy {
            session_verifier: Some(session_verifier),
            ..Default::default()
        };
        p.clients.insert(
            DESKTOP.into(),
            ExchangeClientPolicy {
                public: true,
                session_token_audiences: vec!["knowduet-gateway".into(), "openbot-server".into()],
                ..Default::default()
            },
        );
        p.clients.insert(
            "openbot-server".into(),
            ExchangeClientPolicy {
                on_behalf_of_audiences: vec!["knowduet-gateway".into()],
                requested_claims: vec!["topics".into()],
                ..Default::default()
            },
        );
        p.audiences.insert(
            "openbot-server".into(),
            ExchangeAudiencePolicy {
                copy_claims: vec!["tenant_id".into()],
                ..Default::default()
            },
        );
        p
    }

    async fn minter() -> SharedJwtMinter {
        let cfg = JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("test-secret-key-minimum-length".into()),
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
            ..Default::default()
        };
        Arc::new(RwLock::new(Some(
            JwtMinter::from_config(&cfg).await.unwrap(),
        )))
    }

    /// A subject-token verifier that must never be reached on this path.
    struct NeverVerifier;

    #[async_trait::async_trait]
    impl Authenticator for NeverVerifier {
        async fn authenticate(
            &self,
            _parts: &http::request::Parts,
        ) -> Result<crate::auth::AuthResult, AuthError> {
            panic!("the JWT subject-token verifier must not run for a public client")
        }
    }

    fn session_req(token: &str, audience: &str) -> TokenExchangeRequest {
        TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: token.into(),
            subject_token_type: Some(TOKEN_TYPE_KRATOS_SESSION_TOKEN.into()),
            audience: Some(audience.into()),
            public_client_id: Some(DESKTOP.into()),
            ..Default::default()
        }
    }

    async fn run(req: &TokenExchangeRequest, p: &ExchangePolicy) -> Result<Value, ExchangeError> {
        let verifier: Arc<dyn Authenticator> = Arc::new(NeverVerifier);
        exchange_with_policy(req, &verifier, &minter().await, None, None, p).await
    }

    fn claims(resp: &Value) -> Value {
        let token = resp["access_token"].as_str().unwrap();
        let payload = token.split('.').nth(1).unwrap();
        serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn valid_session_yields_a_token_for_the_person() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let p = policy(kratos(&server.uri()));

        let first = run(&session_req("good-token", "knowduet-gateway"), &p)
            .await
            .expect("valid session");
        let second = run(&session_req("good-token", "knowduet-gateway"), &p)
            .await
            .expect("valid session");

        let c = claims(&first);
        assert_eq!(c["sub"], "identity-7");
        assert_eq!(c["aud"], "knowduet-gateway");
        assert_eq!(c["act"]["sub"], DESKTOP);
        assert_eq!(c["email"], "person@example.test");
        assert_eq!(c["iss"], "flint-gate");
        assert_eq!(c["exp"].as_i64().unwrap() - c["iat"].as_i64().unwrap(), 300);
        assert_eq!(first["expires_in"], 300);
        assert_eq!(first["issued_token_type"], TOKEN_TYPE_ACCESS_TOKEN);
        assert!(c["jti"].as_str().is_some_and(|j| !j.is_empty()));
        assert_ne!(
            c["jti"],
            claims(&second)["jti"],
            "every token has a fresh jti"
        );
        // Untrusted Kratos metadata cannot set the gateway's kind marker.
        assert_eq!(c["flint_kind"], "agent");
    }

    #[tokio::test]
    async fn audience_policy_applies_to_session_tokens() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let resp = run(
            &session_req("good-token", "openbot-server"),
            &policy(kratos(&server.uri())),
        )
        .await
        .unwrap();
        let c = claims(&resp);
        assert_eq!(c["aud"], "openbot-server");
        assert_eq!(c["tenant_id"], "tenant-a");
    }

    #[tokio::test]
    async fn fixed_audience_tenant_and_identity_email_are_on_the_token() {
        // OpenDuet: Hono needs `email`; tenant_id is a fixed per-audience claim.
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let mut p = policy(kratos(&server.uri()));
        p.audiences.get_mut("openbot-server").unwrap().claims =
            json!({ "tenant_id": "deployment-tenant" })
                .as_object()
                .unwrap()
                .clone();
        let resp = run(&session_req("good-token", "openbot-server"), &p)
            .await
            .unwrap();
        let c = claims(&resp);
        assert_eq!(c["sub"], "identity-7");
        assert_eq!(c["email"], "person@example.test");
        // Fixed claim wins over the copied subject claim (same precedence as f9fa42f).
        assert_eq!(c["tenant_id"], "deployment-tenant");
    }

    #[tokio::test]
    async fn expires_in_shortens_and_the_session_expiry_caps_the_lifetime() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let mut r = session_req("good-token", "knowduet-gateway");
        r.expires_in = Some("60".into());
        let resp = run(&r, &policy(kratos(&server.uri()))).await.unwrap();
        assert_eq!(resp["expires_in"], 60);

        let soon = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc3339();
        let server = kratos_with_session(&soon).await;
        let resp = run(
            &session_req("good-token", "knowduet-gateway"),
            &policy(kratos(&server.uri())),
        )
        .await
        .unwrap();
        let ttl = resp["expires_in"].as_u64().unwrap();
        assert!(
            (80..=90).contains(&ttl),
            "ttl {ttl} must not outlive the session"
        );
    }

    #[tokio::test]
    async fn invalid_session_is_401_invalid_grant() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let err = run(
            &session_req("wrong-token", "knowduet-gateway"),
            &policy(kratos(&server.uri())),
        )
        .await
        .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_grant");
        assert_eq!(err.http_status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn expired_session_is_401_invalid_grant() {
        let server = kratos_with_session("2000-01-01T00:00:00Z").await;
        let err = run(
            &session_req("good-token", "knowduet-gateway"),
            &policy(kratos(&server.uri())),
        )
        .await
        .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_grant");
        assert_eq!(err.http_status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unreachable_kratos_is_401_invalid_grant() {
        // Nothing listens on port 1 (wiremock pools dropped servers, so a
        // dropped MockServer's address may still answer).
        let uri = "http://127.0.0.1:1";
        let err = run(
            &session_req("good-token", "knowduet-gateway"),
            &policy(kratos(uri)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_grant");
        assert_eq!(err.http_status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn audience_outside_the_allow_list_is_invalid_target_without_calling_kratos() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let err = run(
            &session_req("good-token", "flint-forge"),
            &policy(kratos(&server.uri())),
        )
        .await
        .unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_target");
    }

    #[tokio::test]
    async fn missing_audience_is_invalid_request() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let mut r = session_req("good-token", "knowduet-gateway");
        r.audience = None;
        let err = run(&r, &policy(kratos(&server.uri()))).await.unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_request");
    }

    #[tokio::test]
    async fn unknown_or_confidential_or_missing_client_id_is_invalid_client() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let p = policy(kratos(&server.uri()));
        for client in [Some("evil-desktop"), Some("openbot-server"), None] {
            let mut r = session_req("good-token", "knowduet-gateway");
            r.public_client_id = client.map(str::to_string);
            let err = run(&r, &p).await.unwrap_err();
            assert_eq!(err, ExchangeError::InvalidClient, "{client:?}");
            assert_eq!(err.http_status(), http::StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn public_client_may_not_present_a_secret() {
        let mut p = policy(kratos("http://127.0.0.1:9"));
        p.client_verifier = Some(Arc::new(AnySecret));
        let mut r = session_req("good-token", "knowduet-gateway");
        r.public_client_id = None;
        r.client_credentials = Some(ClientCredentials {
            client_id: DESKTOP.into(),
            client_secret: "leaked".into(),
        });
        assert_eq!(run(&r, &p).await.unwrap_err(), ExchangeError::InvalidClient);

        // Nor on the plain JWT exchange.
        let mut r = r;
        r.subject_token_type = Some(TOKEN_TYPE_ACCESS_TOKEN.into());
        assert_eq!(run(&r, &p).await.unwrap_err(), ExchangeError::InvalidClient);
    }

    /// A client store that accepts any secret, to prove the public-client
    /// refusal does not depend on the store.
    struct AnySecret;

    #[async_trait::async_trait]
    impl ClientVerifier for AnySecret {
        async fn verify(
            &self,
            client_id: &str,
            _client_secret: &str,
        ) -> anyhow::Result<Option<VerifiedClient>> {
            Ok(Some(VerifiedClient {
                client_id: client_id.into(),
                scopes: vec![],
            }))
        }
    }

    #[tokio::test]
    async fn public_client_may_not_use_any_other_exchange() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let p = policy(kratos(&server.uri()));

        // On-behalf-of.
        let r = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            requested_subject: Some("someone-else".into()),
            audience: Some("knowduet-gateway".into()),
            public_client_id: Some(DESKTOP.into()),
            ..Default::default()
        };
        assert_eq!(
            run(&r, &p).await.unwrap_err().oauth_code(),
            "unauthorized_client"
        );

        // On-behalf-of alongside a session token.
        let mut r = session_req("good-token", "knowduet-gateway");
        r.requested_subject = Some("someone-else".into());
        assert_eq!(
            run(&r, &p).await.unwrap_err().oauth_code(),
            "unauthorized_client"
        );

        // A JWT subject token.
        let mut r = session_req("a.b.c", "knowduet-gateway");
        r.subject_token_type = Some(TOKEN_TYPE_ACCESS_TOKEN.into());
        assert_eq!(
            run(&r, &p).await.unwrap_err().oauth_code(),
            "unauthorized_client"
        );

        // Requested claims.
        let mut r = session_req("good-token", "knowduet-gateway");
        r.extra_fields.insert("topics".into(), "[\"x\"]".into());
        assert_eq!(
            run(&r, &p).await.unwrap_err().oauth_code(),
            "unauthorized_client"
        );

        // Scope (a Kratos session carries none).
        let mut r = session_req("good-token", "knowduet-gateway");
        r.scope = Some("admin".into());
        assert_eq!(run(&r, &p).await.unwrap_err().oauth_code(), "invalid_scope");
    }

    /// Verifies tokens this test minter issued, as the gateway's own `jwt`
    /// subject-token provider would: `sub` is the identity, profile claims are
    /// traits and the rest (including `act`) is metadata.
    struct MintedTokenVerifier;

    #[async_trait::async_trait]
    impl Authenticator for MintedTokenVerifier {
        async fn authenticate(
            &self,
            parts: &http::request::Parts,
        ) -> Result<crate::auth::AuthResult, AuthError> {
            let bearer = parts.headers[http::header::AUTHORIZATION]
                .to_str()
                .unwrap()
                .trim_start_matches("Bearer ");
            let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
            validation.validate_aud = false;
            let mut claims = jsonwebtoken::decode::<Map<String, Value>>(
                bearer,
                &jsonwebtoken::DecodingKey::from_secret(b"test-secret-key-minimum-length"),
                &validation,
            )
            .map_err(|e| AuthError::Unauthorized(e.to_string()))?
            .claims;
            let id = claims.remove("sub").unwrap().as_str().unwrap().to_string();
            let mut traits = Map::new();
            if let Some(email) = claims.remove("email") {
                traits.insert("email".into(), email);
            }
            Ok(crate::auth::AuthResult {
                identity: Identity {
                    id,
                    traits: Value::Object(traits),
                    metadata_public: Value::Object(claims),
                    ..Default::default()
                },
                method: crate::auth::AuthMethod::BearerJwt,
            })
        }
    }

    async fn run_verified(
        req: &TokenExchangeRequest,
        p: &ExchangePolicy,
    ) -> Result<Value, ExchangeError> {
        let verifier: Arc<dyn Authenticator> = Arc::new(MintedTokenVerifier);
        exchange_with_policy(req, &verifier, &minter().await, None, None, p).await
    }

    /// The plain RFC 8693 exchange of `subject_token` for `audience`.
    fn plain_req(subject_token: &str, audience: &str) -> TokenExchangeRequest {
        TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            subject_token: subject_token.into(),
            subject_token_type: Some(TOKEN_TYPE_ACCESS_TOKEN.into()),
            audience: Some(audience.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_public_session_token_cannot_be_exchanged_again() {
        let server = kratos_with_session("2099-01-01T00:00:00Z").await;
        let mut p = policy(kratos(&server.uri()));
        p.client_verifier = Some(Arc::new(AnySecret));

        // The direct session-token exchange still issues the token.
        let minted = run_verified(&session_req("good-token", "knowduet-gateway"), &p)
            .await
            .expect("the public client's own session exchange works");
        let minted = minted["access_token"].as_str().unwrap();

        // Re-presented as a subject token, without and with a confidential
        // client: refused before minting.
        let anonymous = plain_req(minted, "flint-forge");
        let mut confidential = plain_req(minted, "flint-forge");
        confidential.client_credentials = Some(ClientCredentials {
            client_id: "openbot-server".into(),
            client_secret: "pw".into(),
        });
        for r in [anonymous, confidential] {
            let err = run_verified(&r, &p).await.unwrap_err();
            assert!(
                matches!(err, ExchangeError::InvalidSubjectToken(_)),
                "{err:?}"
            );
            assert_eq!(err.oauth_code(), "invalid_request");
            assert_eq!(err.http_status(), http::StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn a_confidential_on_behalf_of_token_is_still_exchangeable() {
        let mut p = policy(kratos("http://127.0.0.1:9"));
        p.client_verifier = Some(Arc::new(AnySecret));
        let obo = TokenExchangeRequest {
            grant_type: GRANT_TYPE_TOKEN_EXCHANGE.into(),
            requested_subject: Some("user-9".into()),
            audience: Some("knowduet-gateway".into()),
            client_credentials: Some(ClientCredentials {
                client_id: "openbot-server".into(),
                client_secret: "pw".into(),
            }),
            ..Default::default()
        };
        let minted = run_verified(&obo, &p).await.expect("on-behalf-of");
        assert_eq!(claims(&minted)["act"]["sub"], "openbot-server");

        let resp = run_verified(
            &plain_req(minted["access_token"].as_str().unwrap(), "flint-forge"),
            &p,
        )
        .await
        .expect("a confidential client's token keeps its current behaviour");
        let c = claims(&resp);
        assert_eq!(c["sub"], "user-9");
        assert_eq!(c["aud"], "flint-forge");
    }

    #[tokio::test]
    async fn session_token_is_never_relayed_to_a_hydra_delegate() {
        let hydra = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(&hydra)
            .await;
        let delegate = HydraDelegate {
            http_client: reqwest::Client::new(),
            token_url: format!("{}/oauth2/token", hydra.uri()),
        };
        let verifier: Arc<dyn Authenticator> = Arc::new(NeverVerifier);
        let err = exchange_with_policy(
            &session_req("good-token", "knowduet-gateway"),
            &verifier,
            &minter().await,
            None,
            Some(&delegate),
            &ExchangePolicy::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExchangeError::InvalidClient);
    }

    // ── Startup policy validation ──────────────────────────────────────────

    fn cfg(client: ExchangeClientPolicy, provider: Option<&str>) -> TokenExchangeConfig {
        TokenExchangeConfig {
            clients: [(DESKTOP.to_string(), client)].into_iter().collect(),
            session_token_provider: provider.map(str::to_string),
            ..Default::default()
        }
    }

    fn desktop() -> ExchangeClientPolicy {
        ExchangeClientPolicy {
            public: true,
            session_token_audiences: vec!["knowduet-gateway".into()],
            ..Default::default()
        }
    }

    #[test]
    fn policy_validation_accepts_a_public_session_client() {
        assert!(validate_exchange_policy(&cfg(desktop(), Some("kratos"))).is_ok());
    }

    #[test]
    fn policy_validation_refuses_mixed_public_policy() {
        let mut obo = desktop();
        obo.on_behalf_of_audiences = vec!["knowduet-gateway".into()];
        let mut claims = desktop();
        claims.requested_claims = vec!["topics".into()];
        let mut none = desktop();
        none.session_token_audiences.clear();
        let mut blank = desktop();
        blank.session_token_audiences = vec![" ".into()];
        let confidential = ExchangeClientPolicy {
            session_token_audiences: vec!["knowduet-gateway".into()],
            ..Default::default()
        };
        for (label, client) in [
            ("on_behalf_of", obo),
            ("requested_claims", claims),
            ("no audiences", none),
            ("blank audience", blank),
            ("confidential with session audiences", confidential),
        ] {
            assert!(
                validate_exchange_policy(&cfg(client, Some("kratos"))).is_err(),
                "{label}"
            );
        }
    }

    #[test]
    fn policy_validation_requires_a_session_token_provider() {
        assert!(validate_exchange_policy(&cfg(desktop(), None)).is_err());
    }
}
