/// Configuration types for Flint Gate.
///
/// All YAML config fields map to these Rust types via serde.
/// Use `#[serde(default)]` liberally for optional fields.
use crate::guardrail::GuardrailHookConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root configuration struct — mirrors the top-level YAML document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GateConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    /// Named auth provider configurations keyed by provider ID.
    #[serde(default)]
    pub auth_providers: HashMap<String, AuthProviderConfig>,
    #[serde(default)]
    pub jwt: JwtConfig,
    #[serde(default)]
    pub sites: Vec<SiteConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    /// OAuth 2.0 Token Exchange (RFC 8693) configuration. Disabled by default.
    #[serde(default)]
    pub token_exchange: TokenExchangeConfig,
    /// OAuth 2.0 features: client-credentials grant + RFC 7662 introspection.
    #[serde(default)]
    pub oauth: OAuthConfig,
}

/// Outcome of [`GateConfig::oauth_exposure_posture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthExposurePosture {
    /// No OAuth endpoint is mounted — nothing to gate.
    NotMounted,
    /// The proxy bind is loopback — local dev, guardrails not enforced.
    AllowLoopback,
    /// Mounted on a non-loopback bind with both introspect-auth and
    /// rate-limiting configured — safe to expose.
    Enforce,
    /// Mounted on a non-loopback bind but missing introspect-auth and/or
    /// rate-limiting — refuse to start (fail-safe against exposing an
    /// under-guarded OAuth surface).
    RefuseStart,
}

impl GateConfig {
    /// Decide whether the `/oauth/*` surface is safe to expose. `/oauth/token`
    /// and `/oauth/introspect` mount on the **proxy** bind (`server.listen`), so
    /// when that bind is non-loopback the endpoints are internet-reachable and
    /// MUST have both introspection client-auth (RFC 7662 §2.1) **and**
    /// rate-limiting enabled. Pure so the fail-closed rule is unit-testable.
    ///
    /// - no OAuth capability mounted            → [`OAuthExposurePosture::NotMounted`]
    /// - loopback proxy bind                    → [`OAuthExposurePosture::AllowLoopback`]
    /// - non-loopback + introspect_auth + limit → [`OAuthExposurePosture::Enforce`]
    /// - non-loopback + missing either guard    → [`OAuthExposurePosture::RefuseStart`]
    pub fn oauth_exposure_posture(&self) -> OAuthExposurePosture {
        let mounted = self.oauth.client_credentials_enabled
            || self.oauth.introspection_enabled
            || self.token_exchange.enabled;
        if !mounted {
            return OAuthExposurePosture::NotMounted;
        }
        if listen_is_loopback(&self.server.listen) {
            return OAuthExposurePosture::AllowLoopback;
        }
        // Non-loopback exposure: require BOTH guards. `introspect_auth` only
        // matters when the introspection endpoint is actually mounted.
        let introspect_guarded =
            !self.oauth.introspection_enabled || self.oauth.introspect_auth;
        let rate_limited = self.oauth.rate_limit.enabled;
        if introspect_guarded && rate_limited {
            OAuthExposurePosture::Enforce
        } else {
            OAuthExposurePosture::RefuseStart
        }
    }
}

/// OAuth 2.0 server-side features flint-gate offers on the proxy port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    /// Enable the `client_credentials` grant on `POST /oauth/token` (service
    /// identities issued from the `oauth_clients` store).
    #[serde(default)]
    pub client_credentials_enabled: bool,
    /// Enable RFC 7662 `POST /oauth/introspect` for gateway-minted tokens.
    #[serde(default)]
    pub introspection_enabled: bool,
    /// Require OAuth **client authentication** on `/oauth/introspect`
    /// (`client_id` + `client_secret`, HTTP Basic or form), verified against the
    /// `oauth_clients` store. RFC 7662 §2.1 makes this a MUST (token-scanning
    /// defense), so it defaults to **true** — set it `false` ONLY when the
    /// endpoint is network-restricted to trusted callers.
    #[serde(default = "default_true")]
    pub introspect_auth: bool,
    /// Per-endpoint rate limiting for `/oauth/token` and `/oauth/introspect`,
    /// independent of the proxy `server.rate_limit`. When a shared Redis limiter
    /// is available it is used (authoritative across replicas); otherwise the
    /// in-process governor applies. Applied as a tower layer on the OAuth
    /// sub-router.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// What to do when the **shared** rate-limit backend (Redis) is unavailable
    /// mid-request. Fail-closed by default: the introspection endpoint is a
    /// token-scanning oracle (RFC 7662 §2.1), so losing its limit must **deny**.
    /// The token endpoint's wiring may *degrade* to the in-process governor to
    /// avoid an availability cliff — but an operator can force uniform `Deny`
    /// here. Only consulted when a shared limiter is configured.
    #[serde(default)]
    pub on_backend_unavailable: BackendUnavailablePosture,
    /// Default TTL (seconds) for client-credentials service tokens.
    #[serde(default)]
    pub service_token_ttl_seconds: Option<u64>,
    /// **Seam (off by default):** delegate introspection of opaque, Hydra-issued
    /// tokens to Ory Hydra's admin introspection endpoint. When set, tokens that
    /// do not verify as gateway-minted are forwarded here (Hydra owns RFC 7662
    /// for its own tokens).
    ///
    /// SECURITY: this delegate proxies to Hydra's *admin* API, so it is only
    /// reachable **after** `/oauth/introspect` client authentication passes
    /// (`introspect_auth`, default true). Keep `introspect_auth` on when a
    /// delegate is configured; disabling it exposes Hydra's admin surface.
    #[serde(default)]
    pub introspection_delegate: Option<IntrospectionDelegateConfig>,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        // `introspect_auth` defaults to TRUE (fail-closed / RFC 7662 MUST) on
        // EVERY construction path, including `..Default::default()` — a derived
        // Default would give `false` and silently disable introspection auth.
        Self {
            client_credentials_enabled: false,
            introspection_enabled: false,
            introspect_auth: true,
            rate_limit: RateLimitConfig::default(),
            on_backend_unavailable: BackendUnavailablePosture::default(),
            service_token_ttl_seconds: None,
            introspection_delegate: None,
        }
    }
}

/// Posture when the shared (Redis) rate-limit backend is unavailable mid-request.
///
/// Defaults to [`Deny`](BackendUnavailablePosture::Deny) — fail-closed. The
/// token endpoint may override to `Degrade` (fall back to the in-process
/// governor + a WARN); the introspection oracle should stay `Deny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendUnavailablePosture {
    /// Deny the request when the shared limiter cannot be consulted (fail-closed).
    #[default]
    Deny,
    /// Fall back to the in-process governor and emit a WARN (availability-first).
    Degrade,
}

/// Configuration for delegating introspection to an external AS (Ory Hydra).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionDelegateConfig {
    /// Hydra admin base URL exposing `POST /admin/oauth2/introspect`.
    pub hydra_admin_url: String,
}

/// OAuth 2.0 Token Exchange (RFC 8693) settings.
///
/// flint-gate performs *gateway-local* exchange: it verifies an incoming
/// `subject_token` against a configured auth provider's JWKS (so **any IdM that
/// issues a verifiable JWT** is a valid subject-token source), downscopes, and
/// mints a delegated token carrying an `act` claim. `delegate_to_hydra` is a
/// forward-looking seam for proxying the exchange to an Ory Hydra token endpoint
/// — **not yet implemented**; when true today the exchange still runs locally.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenExchangeConfig {
    /// Enable the `POST /oauth/token` token-exchange endpoint.
    #[serde(default)]
    pub enabled: bool,
    /// Name of the `auth_providers` entry used to verify the `subject_token`.
    /// Must be a JWKS-backed provider (`jwt` or `mcp`). Required when `enabled`.
    #[serde(default)]
    pub subject_token_provider: Option<String>,
    /// TTL (seconds) for minted delegated tokens. Falls back to the JWT default.
    #[serde(default)]
    pub delegated_ttl_seconds: Option<u64>,
    /// Federate-first: proxy the exchange to an Ory Hydra token endpoint (Hydra
    /// owns RFC 8693) instead of minting locally. Requires `hydra_token_url`.
    /// Fails closed (deny) on a Hydra transport/non-2xx error — no local fallback.
    /// CAVEAT: Hydra has known external-token `aud` quirks (ory/hydra#3723).
    #[serde(default)]
    pub delegate_to_hydra: bool,
    /// The Hydra token endpoint used when `delegate_to_hydra` is set.
    #[serde(default)]
    pub hydra_token_url: Option<String>,
}

/// HTTP server bind configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Proxy server listen address (default: `0.0.0.0:4456`).
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Admin API listen address (default: `127.0.0.1:4457` — LOOPBACK ONLY).
    ///
    /// The admin API is unauthenticated and MUST NEVER be internet-exposed. It
    /// defaults to loopback so operators must opt in explicitly (e.g. bind
    /// `0.0.0.0` behind a firewall / private network) rather than accidentally
    /// exposing route, key, and policy management to the public internet.
    #[serde(default = "default_admin_listen")]
    pub admin_listen: String,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Seconds to wait for in-flight connections to finish draining on shutdown.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
    /// In-process per-replica request-rate limiter (coarse burst shield).
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    /// Authentication for the admin API. When `None`, the admin API is
    /// unauthenticated and only permitted on a loopback `admin_listen` — binding
    /// a non-loopback address without `admin_auth` is refused at startup.
    #[serde(default)]
    pub admin_auth: Option<AdminAuthConfig>,
    /// Allow plaintext `http://` upstream URLs (Hydra token/admin endpoints).
    /// **Off by default** — a plaintext upstream carrying a `subject_token` or
    /// client credentials is refused at startup. Set true ONLY for local dev
    /// against an `http://` Hydra; a loud WARN is emitted when enabled.
    #[serde(default)]
    pub allow_insecure_upstream: bool,
}

/// Validate that an operator-configured upstream URL uses `https://` unless
/// `allow_insecure_upstream` is set. Pure so the fail-closed rule is
/// unit-testable. Returns `Err(reason)` when a plaintext URL is refused.
///
/// `https://` → always Ok. `http://` → Ok only when `allow` is true (caller
/// should WARN). Any other scheme (or none) is refused — an upstream that
/// forwards credentials must be an explicit, TLS-protected URL.
pub fn validate_upstream_url_scheme(
    field: &str,
    url: &str,
    allow_insecure: bool,
) -> Result<(), String> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("https://") {
        Ok(())
    } else if lower.starts_with("http://") {
        if allow_insecure {
            Ok(())
        } else {
            Err(format!(
                "{field} is a plaintext http:// URL ({url}); refusing to forward \
                 credentials over cleartext. Use https:// or set \
                 server.allow_insecure_upstream: true for local development."
            ))
        }
    } else {
        Err(format!(
            "{field} must be an https:// URL (got {url:?})"
        ))
    }
}

/// Authentication policy for the admin API.
///
/// Reuses an existing [`AuthProviderConfig`] (JWT or Kratos are the intended
/// choices — the Ory-standard path; any JWKS-backed JWT provider also works) so
/// there is no separate admin identity model. When set, every admin request
/// except the liveness/readiness probes MUST authenticate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAuthConfig {
    /// The provider used to verify admin requests. Inline config; typically
    /// `type: jwt` (Bearer) or `type: kratos` (session cookie).
    pub provider: AuthProviderConfig,
}

impl ServerConfig {
    /// Decide the admin-API auth posture from whether `admin_auth` is configured
    /// and whether `admin_listen` binds a loopback address. Pure and
    /// side-effect free so the fail-closed rule is unit-testable.
    ///
    /// - `admin_auth` set                         → [`AdminAuthPosture::Enforce`]
    /// - unset **and** loopback bind              → [`AdminAuthPosture::AllowLoopback`]
    /// - unset **and** non-loopback bind          → [`AdminAuthPosture::RefuseStart`]
    pub fn admin_auth_posture(&self) -> AdminAuthPosture {
        if self.admin_auth.is_some() {
            return AdminAuthPosture::Enforce;
        }
        if listen_is_loopback(&self.admin_listen) {
            AdminAuthPosture::AllowLoopback
        } else {
            AdminAuthPosture::RefuseStart
        }
    }
}

/// Outcome of [`ServerConfig::admin_auth_posture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAuthPosture {
    /// `admin_auth` is configured — authenticate every admin request.
    Enforce,
    /// No auth, but the bind is loopback — permitted for local development.
    AllowLoopback,
    /// No auth on a non-loopback bind — refuse to start (fail-safe against
    /// exposing an unauthenticated control plane).
    RefuseStart,
}

/// Whether a `host:port` bind value binds a loopback address. Treats a missing /
/// unparseable host conservatively as **non-loopback** so an ambiguous bind
/// fails safe (refuse-start) rather than being silently permitted. Used for both
/// the admin bind and the proxy (`server.listen`) bind that hosts `/oauth/*`.
fn listen_is_loopback(listen: &str) -> bool {
    // Split host from the trailing `:port`. IPv6 literals are bracketed
    // (`[::1]:4457`); IPv4/hostname take the substring before the last colon.
    let host = if let Some(rest) = listen.strip_prefix('[') {
        // `[::1]:4457` → `::1`
        match rest.split_once(']') {
            Some((h, _)) => h,
            None => return false,
        }
    } else {
        listen.rsplit_once(':').map(|(h, _)| h).unwrap_or(listen)
    };

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// In-process request-rate limiting via `tower_governor`.
///
/// This is the per-replica, in-memory burst shield keyed on the API key /
/// identity (falling back to peer IP). It is intentionally coarse — the
/// authoritative, cross-replica limiting lives in the Redis window counters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Enable the in-process request-rate layer on the proxy router.
    #[serde(default)]
    pub enabled: bool,
    /// Sustained requests-per-second replenishment rate per key.
    #[serde(default = "default_rate_per_second")]
    pub per_second: u64,
    /// Maximum burst size (bucket capacity) per key.
    #[serde(default = "default_rate_burst")]
    pub burst: u32,
}

fn default_rate_per_second() -> u64 {
    50
}
fn default_rate_burst() -> u32 {
    100
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            per_second: default_rate_per_second(),
            burst: default_rate_burst(),
        }
    }
}

fn default_listen() -> String {
    "0.0.0.0:4456".to_string()
}
fn default_admin_listen() -> String {
    // Loopback by default: the admin API is unauthenticated and must be
    // firewalled / kept off the public internet. Operators opt into wider
    // exposure explicitly.
    "127.0.0.1:4457".to_string()
}
fn default_shutdown_timeout() -> u64 {
    30
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            admin_listen: default_admin_listen(),
            tls: TlsConfig::default(),
            shutdown_timeout_secs: default_shutdown_timeout(),
            rate_limit: RateLimitConfig::default(),
            admin_auth: None,
            allow_insecure_upstream: false,
        }
    }
}

/// TLS termination settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

/// Postgres database connection settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Postgres connection URL. If empty, DB features are disabled.
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// When true, DB routes take precedence over YAML routes.
    #[serde(default)]
    pub override_yaml: bool,
}

fn default_max_connections() -> u32 {
    20
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: default_max_connections(),
            override_yaml: false,
        }
    }
}

/// Cache configuration (moka L1 + optional Redis L2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default)]
    pub l1: L1CacheConfig,
    #[serde(default)]
    pub l2: L2CacheConfig,
    /// Postgres LISTEN channel name for cache invalidation events.
    #[serde(default = "default_invalidation_channel")]
    pub invalidation_channel: String,
}

fn default_invalidation_channel() -> String {
    "flintgate_config_changed".to_string()
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            l1: L1CacheConfig::default(),
            l2: L2CacheConfig::default(),
            invalidation_channel: default_invalidation_channel(),
        }
    }
}

/// Moka in-process cache settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1CacheConfig {
    #[serde(default = "default_l1_max_capacity")]
    pub max_capacity: u64,
    #[serde(default = "default_l1_ttl")]
    pub ttl_seconds: u64,
}

fn default_l1_max_capacity() -> u64 {
    10_000
}
fn default_l1_ttl() -> u64 {
    60
}

impl Default for L1CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: default_l1_max_capacity(),
            ttl_seconds: default_l1_ttl(),
        }
    }
}

/// Optional Redis L2 cache.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct L2CacheConfig {
    #[serde(default)]
    pub enabled: bool,
    pub redis_url: Option<String>,
}

/// Discriminated union over the supported auth provider types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthProviderConfig {
    Kratos(KratosAuthConfig),
    Jwt(JwtAuthConfig),
    ApiKey(ApiKeyAuthConfig),
    Anonymous(AnonymousAuthConfig),
    /// MCP-era OAuth 2.1 Resource Server (RFC 8707 audience binding, scope
    /// enforcement, RFC 9728 metadata discovery). Superset of `Jwt`.
    Mcp(McpAuthConfig),
}

/// Ory Kratos session authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KratosAuthConfig {
    pub base_url: String,
    /// Forward the incoming session cookie to Kratos.
    #[serde(default = "default_true")]
    pub forward_cookies: bool,
    #[serde(default = "default_session_cookie")]
    pub session_cookie: String,
}

fn default_session_cookie() -> String {
    "ory_kratos_session".to_string()
}
fn default_true() -> bool {
    true
}

/// Inbound JWT Bearer verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtAuthConfig {
    pub jwks_url: String,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    #[serde(default = "default_leeway")]
    pub leeway_seconds: u64,
}

fn default_leeway() -> u64 {
    5
}

/// MCP OAuth 2.1 Resource Server authentication.
///
/// This RS validates access tokens minted by an external Authorization Server.
/// It NEVER acts as an AS itself. The security crux is RFC 8707: a token whose
/// `aud` does not include this RS's `audience` MUST be rejected even when its
/// signature is valid (confused-deputy defense).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthConfig {
    /// JWKS endpoint of the trusted Authorization Server.
    pub jwks_url: String,
    /// Expected `iss` claim. When set, tokens with a different issuer are rejected.
    #[serde(default)]
    pub issuer: Option<String>,
    /// RFC 8707 resource identifier this RS accepts in the token `aud`.
    /// When set, the token's audience MUST include this value.
    #[serde(default)]
    pub audience: Option<String>,
    /// This server's canonical resource URI, advertised in the RFC 9728
    /// Protected Resource Metadata document.
    pub resource: String,
    /// Authorization Server issuer URLs advertised in the metadata document.
    #[serde(default)]
    pub authorization_servers: Vec<String>,
    /// Scopes the caller's token MUST carry (superset check). Empty = no scope gate.
    #[serde(default)]
    pub required_scopes: Vec<String>,
    #[serde(default = "default_leeway")]
    pub leeway_seconds: u64,
}

/// API key header extraction + database lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyAuthConfig {
    #[serde(default = "default_api_key_header")]
    pub header: String,
    #[serde(default = "default_api_key_store")]
    pub store: String,
}

fn default_api_key_header() -> String {
    "X-API-Key".to_string()
}
fn default_api_key_store() -> String {
    "database".to_string()
}

/// Anonymous / passthrough authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnonymousAuthConfig {
    #[serde(default = "default_anonymous_subject")]
    pub default_subject: String,
}

fn default_anonymous_subject() -> String {
    "anonymous".to_string()
}

impl Default for AnonymousAuthConfig {
    fn default() -> Self {
        Self {
            default_subject: default_anonymous_subject(),
        }
    }
}

/// Outbound JWT minting configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    #[serde(default = "default_jwt_algorithm")]
    pub signing_algorithm: String,
    /// Path to PEM-encoded private key file (RS256/ES256).
    pub signing_key_path: Option<String>,
    /// Raw HMAC secret (HS256). Prefer `signing_key_path` for production.
    pub signing_key_secret: Option<String>,
    #[serde(default = "default_jwt_issuer")]
    pub issuer: String,
    #[serde(default = "default_jwt_ttl")]
    pub default_ttl_seconds: u64,
}

fn default_jwt_algorithm() -> String {
    "HS256".to_string()
}
fn default_jwt_issuer() -> String {
    "flint-gate".to_string()
}
fn default_jwt_ttl() -> u64 {
    300
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            signing_algorithm: default_jwt_algorithm(),
            signing_key_path: None,
            signing_key_secret: None,
            issuer: default_jwt_issuer(),
            default_ttl_seconds: default_jwt_ttl(),
        }
    }
}

/// A logical site — maps one or more domains to a set of routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteConfig {
    pub id: String,
    #[serde(default)]
    pub domains: Vec<String>,
    /// Default auth provider ID for routes in this site.
    pub default_auth: Option<String>,
    /// Base upstream URL when a route doesn't specify one.
    pub default_upstream: Option<String>,
}

/// A single proxied route definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    pub id: String,
    pub site: String,
    #[serde(rename = "match")]
    pub route_match: RouteMatch,
    /// Full upstream URL for this route (overrides site default).
    pub upstream: Option<String>,
    /// Auth provider ID (overrides site default).
    pub auth: Option<String>,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub stream: StreamConfig,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Route matching criteria.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouteMatch {
    /// Glob path pattern, e.g. `/api/**` or `/health`.
    pub path: String,
    /// HTTP methods to match. Empty means all methods.
    #[serde(default)]
    pub methods: Vec<String>,
    /// Optional host pattern to restrict this route to a specific subdomain.
    pub host: Option<String>,
}

/// Pre-request and post-response hook chains for a route.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default)]
    pub pre_request: Vec<PreRequestHook>,
    #[serde(default)]
    pub post_response: Vec<PostResponseHook>,
}

/// A single pre-request hook step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreRequestHook {
    /// Inject headers and optionally mint an outbound JWT.
    ClaimsEnhancement { config: ClaimsEnhancementConfig },
    /// Modify JSON fields in the request body.
    BodyTransform { config: BodyTransformConfig },
    /// Block the request if the user's lifetime token usage exceeds a limit.
    MaxTokenBudget { config: MaxTokenBudgetConfig },
    /// Evaluate an embedded Cedar authorization policy for this route.
    Authorize { config: AuthorizeConfig },
    /// Inspect the request with a guardrail and optionally block it.
    Guardrail { config: GuardrailHookConfig },
}

/// A single post-response hook step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PostResponseHook {
    /// Record stream metrics to the usage_events table.
    StreamMeter { config: StreamMeterConfig },
}

/// Configuration for the claims_enhancement hook.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaimsEnhancementConfig {
    /// Header name → template expression mapping.
    #[serde(default)]
    pub inject_headers: HashMap<String, String>,
    pub mint_jwt: Option<MintJwtConfig>,
}

/// JWT minting sub-config within claims_enhancement.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MintJwtConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Additional claims merged into the minted JWT payload.
    #[serde(default)]
    pub additional_claims: serde_json::Value,
}

/// Configuration for the body_transform hook.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BodyTransformConfig {
    /// JSON field path → template expression.
    #[serde(default)]
    pub set_fields: HashMap<String, String>,
}

/// Configuration for the `authorize` pre-request hook (Cedar policy engine).
///
/// The engine models actions generically: this hook contributes the request's
/// `principal` (the authenticated identity), a single generic `action` (default
/// `"invoke"`), the matched `resource` (the route), and a `context` record built
/// from request attributes. Per-tool-call decisions are a later change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizeConfig {
    /// Generic Cedar action id to evaluate. Defaults to `"invoke"`.
    #[serde(default = "default_authorize_action")]
    pub action: String,
    /// When `false`, a `Deny` decision is logged but the request is allowed to
    /// proceed (audit/shadow mode). Defaults to `true` (enforce → 403 on deny).
    #[serde(default = "default_true")]
    pub enforce: bool,
    /// Custom message returned in the 403 body on a denied request.
    #[serde(default)]
    pub error_message: Option<String>,
}

fn default_authorize_action() -> String {
    "invoke".to_string()
}

impl Default for AuthorizeConfig {
    fn default() -> Self {
        Self {
            action: default_authorize_action(),
            enforce: true,
            error_message: None,
        }
    }
}

/// Configuration for the max_token_budget pre-request hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaxTokenBudgetConfig {
    /// Maximum tokens allowed within `window`. Requests exceeding this return 429.
    pub limit: u64,
    /// Template expression that resolves to the user identifier.
    #[serde(default = "default_user_id_expr")]
    pub user_id_expr: String,
    /// Custom error message in the 429 response body.
    pub error_message: Option<String>,
    /// Budget accounting window. `lifetime` (default) preserves the original
    /// behavior of summing all-time usage from the `usage_events` ledger.
    #[serde(default)]
    pub window: BudgetWindow,
    /// Whether the budget is accounted per-user or per-team.
    #[serde(default)]
    pub scope: BudgetScope,
}

fn default_user_id_expr() -> String {
    "identity.id".to_string()
}

/// The accounting window for a token budget.
///
/// `Lifetime` is the default and reproduces the pre-windowing behavior
/// (all-time sum from `usage_events`). The fixed windows are enforced via
/// Redis fixed-window counters (or a Postgres time-bounded sum fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BudgetWindow {
    /// All-time cumulative usage (original behavior).
    #[default]
    Lifetime,
    /// Rolling per-minute fixed window.
    Minute,
    /// Rolling per-hour fixed window.
    Hour,
    /// Rolling per-day fixed window.
    Day,
}

impl BudgetWindow {
    /// The fixed-window length in seconds, or `None` for `Lifetime`.
    pub fn duration_secs(&self) -> Option<u64> {
        match self {
            BudgetWindow::Lifetime => None,
            BudgetWindow::Minute => Some(60),
            BudgetWindow::Hour => Some(3_600),
            BudgetWindow::Day => Some(86_400),
        }
    }

    /// A short, stable string tag used in Redis keys and Postgres intervals.
    pub fn tag(&self) -> &'static str {
        match self {
            BudgetWindow::Lifetime => "lifetime",
            BudgetWindow::Minute => "minute",
            BudgetWindow::Hour => "hour",
            BudgetWindow::Day => "day",
        }
    }

    /// Postgres interval literal for the windowed fallback query.
    /// Returns `None` for `Lifetime` (no time bound).
    pub fn pg_interval(&self) -> Option<&'static str> {
        match self {
            BudgetWindow::Lifetime => None,
            BudgetWindow::Minute => Some("1 minute"),
            BudgetWindow::Hour => Some("1 hour"),
            BudgetWindow::Day => Some("1 day"),
        }
    }
}

/// The subject a budget is accounted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    /// Per-user accounting (default).
    #[default]
    User,
    /// Per-team accounting.
    Team,
}

impl BudgetScope {
    /// A short, stable string tag used in Redis keys.
    pub fn tag(&self) -> &'static str {
        match self {
            BudgetScope::User => "user",
            BudgetScope::Team => "team",
        }
    }
}

/// Configuration for the stream_meter post-response hook.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamMeterConfig {
    #[serde(default = "default_true")]
    pub log_to_db: bool,
}

/// SSE/WebSocket streaming configuration for a route.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamConfig {
    #[serde(default)]
    pub enabled: bool,
    /// `sse`, `websocket`, or `ndjson`.
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub ai: AiStreamConfig,
}

fn default_protocol() -> String {
    "sse".to_string()
}

/// AI protocol processing configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiStreamConfig {
    #[serde(default)]
    pub ag_ui: AgUiConfig,
    #[serde(default)]
    pub a2ui: A2UiConfig,
    pub session_watchdog: Option<SessionWatchdogConfig>,
    #[serde(default)]
    pub backpressure: BackpressureConfig,
}

/// AG-UI (CopilotKit) event processing settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgUiConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Drop events not in `allowed_events`.
    #[serde(default)]
    pub validate_events: bool,
    #[serde(default)]
    pub allowed_events: Vec<String>,
    /// Template expressions injected into every event's `_gate_metadata`.
    #[serde(default)]
    pub inject_metadata: HashMap<String, String>,
}

/// A2UI intent-driven UI protocol settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct A2UiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_intents: Vec<String>,
    /// Theme object injected into `render_component` payloads as `_theme`.
    #[serde(default)]
    pub theme: Option<serde_json::Value>,
}

/// Session watchdog configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWatchdogConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_watchdog_interval")]
    pub check_interval_seconds: u64,
}

fn default_watchdog_interval() -> u64 {
    30
}

/// Stream backpressure / circuit-breaking limits.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackpressureConfig {
    pub max_stream_duration_seconds: Option<u64>,
    pub max_events: Option<u64>,
    /// Cap (bytes) on a single buffered SSE/NDJSON event's assembled payload
    /// (and the raw line buffer). Exceeding this terminates the stream — a
    /// guard against unbounded-buffering DoS (C1). `None` → built-in default
    /// [`crate::stream::DEFAULT_MAX_EVENT_BYTES`].
    #[serde(default)]
    pub max_event_bytes: Option<usize>,
    /// Cap (bytes) on the accumulated arguments of a single tool call buffered
    /// pending authorization. Exceeding it denies that tool call (drop + emit a
    /// RUN_ERROR) without tearing down the whole stream. `None` → built-in
    /// default [`crate::stream::DEFAULT_MAX_TOOL_ARGS_BYTES`].
    #[serde(default)]
    pub max_tool_args_bytes: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.listen, "0.0.0.0:4456");
        // Admin API defaults to LOOPBACK — it is unauthenticated and must never
        // be internet-exposed (H3). Operators opt into wider binds explicitly.
        assert_eq!(cfg.admin_listen, "127.0.0.1:4457");
    }

    #[test]
    fn deserialize_auth_provider_kratos() {
        let yaml = r#"
type: kratos
base_url: "http://kratos:4433"
"#;
        let provider: AuthProviderConfig = serde_yaml::from_str(yaml).unwrap();
        match provider {
            AuthProviderConfig::Kratos(cfg) => {
                assert_eq!(cfg.base_url, "http://kratos:4433");
            }
            _ => panic!("expected Kratos"),
        }
    }

    #[test]
    fn deserialize_anonymous_auth() {
        let yaml = r#"type: anonymous"#;
        let provider: AuthProviderConfig = serde_yaml::from_str(yaml).unwrap();
        match provider {
            AuthProviderConfig::Anonymous(cfg) => {
                assert_eq!(cfg.default_subject, "anonymous");
            }
            _ => panic!("expected Anonymous"),
        }
    }

    #[test]
    fn deserialize_mcp_auth_full() {
        let yaml = r#"
type: mcp
jwks_url: "https://as.example/.well-known/jwks.json"
issuer: "https://as.example"
audience: "https://gate.example/mcp"
resource: "https://gate.example/mcp"
authorization_servers:
  - "https://as.example"
required_scopes:
  - "mcp:read"
  - "mcp:write"
leeway_seconds: 10
"#;
        let provider: AuthProviderConfig = serde_yaml::from_str(yaml).unwrap();
        match provider {
            AuthProviderConfig::Mcp(cfg) => {
                assert_eq!(cfg.jwks_url, "https://as.example/.well-known/jwks.json");
                assert_eq!(cfg.issuer.as_deref(), Some("https://as.example"));
                assert_eq!(cfg.audience.as_deref(), Some("https://gate.example/mcp"));
                assert_eq!(cfg.resource, "https://gate.example/mcp");
                assert_eq!(cfg.authorization_servers, vec!["https://as.example"]);
                assert_eq!(cfg.required_scopes, vec!["mcp:read", "mcp:write"]);
                assert_eq!(cfg.leeway_seconds, 10);
            }
            _ => panic!("expected Mcp"),
        }
    }

    #[test]
    fn deserialize_mcp_auth_defaults() {
        // Only the required fields; optional fields default (empty vecs, None,
        // leeway = 5). This exercises the fail-open-only-where-safe defaults.
        let yaml = r#"
type: mcp
jwks_url: "https://as.example/jwks"
resource: "https://gate.example/mcp"
"#;
        let provider: AuthProviderConfig = serde_yaml::from_str(yaml).unwrap();
        match provider {
            AuthProviderConfig::Mcp(cfg) => {
                assert!(cfg.issuer.is_none());
                assert!(cfg.audience.is_none());
                assert!(cfg.authorization_servers.is_empty());
                assert!(cfg.required_scopes.is_empty());
                assert_eq!(cfg.leeway_seconds, 5);
            }
            _ => panic!("expected Mcp"),
        }
    }

    // ── Task 2: MaxTokenBudget window/scope backward compatibility ──────────

    #[test]
    fn legacy_max_token_budget_yaml_defaults_to_lifetime_user() {
        // Arrange — a pre-windowing config with only limit + user_id_expr.
        let yaml = r#"
limit: 100000
user_id_expr: "identity.id"
"#;
        // Act
        let cfg: MaxTokenBudgetConfig = serde_yaml::from_str(yaml).unwrap();
        // Assert — serde defaults preserve the original all-time, per-user semantics.
        assert_eq!(cfg.limit, 100_000);
        assert_eq!(cfg.user_id_expr, "identity.id");
        assert_eq!(cfg.window, BudgetWindow::Lifetime);
        assert_eq!(cfg.scope, BudgetScope::User);
        assert!(cfg.error_message.is_none());
    }

    #[test]
    fn minimal_max_token_budget_yaml_only_limit_still_deserializes() {
        // Arrange — the absolute minimum: user_id_expr also defaults.
        let yaml = r#"limit: 42"#;
        // Act
        let cfg: MaxTokenBudgetConfig = serde_yaml::from_str(yaml).unwrap();
        // Assert
        assert_eq!(cfg.limit, 42);
        assert_eq!(cfg.user_id_expr, "identity.id");
        assert_eq!(cfg.window, BudgetWindow::Lifetime);
        assert_eq!(cfg.scope, BudgetScope::User);
    }

    #[test]
    fn windowed_max_token_budget_yaml_deserializes() {
        // Arrange — new-style config selecting an hourly, team-scoped budget.
        let yaml = r#"
limit: 5000
window: hour
scope: team
error_message: "team hourly cap reached"
"#;
        // Act
        let cfg: MaxTokenBudgetConfig = serde_yaml::from_str(yaml).unwrap();
        // Assert
        assert_eq!(cfg.limit, 5000);
        assert_eq!(cfg.window, BudgetWindow::Hour);
        assert_eq!(cfg.scope, BudgetScope::Team);
        assert_eq!(
            cfg.error_message.as_deref(),
            Some("team hourly cap reached")
        );
    }

    #[test]
    fn max_token_budget_as_pre_request_hook_variant() {
        // Arrange — full hook shape with the snake_case tag.
        let yaml = r#"
type: max_token_budget
config:
  limit: 10
  window: day
  scope: user
"#;
        // Act
        let hook: PreRequestHook = serde_yaml::from_str(yaml).unwrap();
        // Assert
        match hook {
            PreRequestHook::MaxTokenBudget { config } => {
                assert_eq!(config.window, BudgetWindow::Day);
                assert_eq!(config.scope, BudgetScope::User);
            }
            _ => panic!("expected MaxTokenBudget"),
        }
    }

    #[test]
    fn budget_window_serde_round_trip() {
        for (window, tag) in [
            (BudgetWindow::Lifetime, "lifetime"),
            (BudgetWindow::Minute, "minute"),
            (BudgetWindow::Hour, "hour"),
            (BudgetWindow::Day, "day"),
        ] {
            // snake_case tag matches the enum's `tag()` helper.
            assert_eq!(window.tag(), tag);
            let json = serde_json::to_string(&window).unwrap();
            assert_eq!(json, format!("\"{tag}\""));
            let back: BudgetWindow = serde_json::from_str(&json).unwrap();
            assert_eq!(back, window);
        }
    }

    #[test]
    fn budget_scope_serde_round_trip() {
        for (scope, tag) in [(BudgetScope::User, "user"), (BudgetScope::Team, "team")] {
            assert_eq!(scope.tag(), tag);
            let json = serde_json::to_string(&scope).unwrap();
            assert_eq!(json, format!("\"{tag}\""));
            let back: BudgetScope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
        }
    }

    #[test]
    fn budget_window_duration_and_interval_mapping() {
        // Fixed windows expose both a TTL (seconds) and a Postgres interval.
        assert_eq!(BudgetWindow::Lifetime.duration_secs(), None);
        assert_eq!(BudgetWindow::Minute.duration_secs(), Some(60));
        assert_eq!(BudgetWindow::Hour.duration_secs(), Some(3_600));
        assert_eq!(BudgetWindow::Day.duration_secs(), Some(86_400));

        assert_eq!(BudgetWindow::Lifetime.pg_interval(), None);
        assert_eq!(BudgetWindow::Minute.pg_interval(), Some("1 minute"));
        assert_eq!(BudgetWindow::Hour.pg_interval(), Some("1 hour"));
        assert_eq!(BudgetWindow::Day.pg_interval(), Some("1 day"));
    }

    #[test]
    fn budget_defaults_are_lifetime_and_user() {
        assert_eq!(BudgetWindow::default(), BudgetWindow::Lifetime);
        assert_eq!(BudgetScope::default(), BudgetScope::User);
    }

    // ── Authorize pre-request hook (Cedar policy engine) ───────────────────

    #[test]
    fn authorize_hook_deserializes_with_defaults() {
        // Minimal config: action defaults to "invoke", enforce defaults to true.
        let yaml = r#"
type: authorize
config: {}
"#;
        let hook: PreRequestHook = serde_yaml::from_str(yaml).unwrap();
        match hook {
            PreRequestHook::Authorize { config } => {
                assert_eq!(config.action, "invoke");
                assert!(config.enforce, "enforce must default to true (fail-closed)");
                assert!(config.error_message.is_none());
            }
            _ => panic!("expected Authorize"),
        }
    }

    #[test]
    fn authorize_hook_deserializes_full_config() {
        let yaml = r#"
type: authorize
config:
  action: read
  enforce: false
  error_message: "not allowed here"
"#;
        let hook: PreRequestHook = serde_yaml::from_str(yaml).unwrap();
        match hook {
            PreRequestHook::Authorize { config } => {
                assert_eq!(config.action, "read");
                assert!(!config.enforce);
                assert_eq!(config.error_message.as_deref(), Some("not allowed here"));
            }
            _ => panic!("expected Authorize"),
        }
    }

    #[test]
    fn authorize_config_default_is_enforcing_invoke() {
        let cfg = AuthorizeConfig::default();
        assert_eq!(cfg.action, "invoke");
        assert!(cfg.enforce);
    }

    // ── Task 3: RateLimitConfig defaults ──────────────────────────────────

    #[test]
    fn rate_limit_config_defaults_disabled_with_sane_rate() {
        let cfg = RateLimitConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.per_second, 50);
        assert_eq!(cfg.burst, 100);
    }

    #[test]
    fn server_config_includes_default_rate_limit() {
        let cfg = ServerConfig::default();
        assert!(!cfg.rate_limit.enabled);
    }

    // ── Admin-auth posture (fail-closed startup guard) ────────────────────

    fn server_with(admin_listen: &str, auth: Option<AdminAuthConfig>) -> ServerConfig {
        ServerConfig {
            admin_listen: admin_listen.to_string(),
            admin_auth: auth,
            ..ServerConfig::default()
        }
    }

    fn dummy_admin_auth() -> AdminAuthConfig {
        AdminAuthConfig {
            provider: AuthProviderConfig::Jwt(JwtAuthConfig {
                jwks_url: "https://issuer.example/.well-known/jwks.json".into(),
                issuer: Some("https://issuer.example".into()),
                audience: Some("flint-admin".into()),
                leeway_seconds: default_leeway(),
            }),
        }
    }

    #[test]
    fn posture_enforces_when_admin_auth_set_regardless_of_bind() {
        // auth set + loopback → enforce; auth set + public → enforce.
        assert_eq!(
            server_with("127.0.0.1:4457", Some(dummy_admin_auth())).admin_auth_posture(),
            AdminAuthPosture::Enforce
        );
        assert_eq!(
            server_with("0.0.0.0:4457", Some(dummy_admin_auth())).admin_auth_posture(),
            AdminAuthPosture::Enforce
        );
    }

    #[test]
    fn posture_allows_loopback_without_auth() {
        for addr in ["127.0.0.1:4457", "[::1]:4457", "localhost:4457"] {
            assert_eq!(
                server_with(addr, None).admin_auth_posture(),
                AdminAuthPosture::AllowLoopback,
                "{addr} should be treated as loopback"
            );
        }
    }

    #[test]
    fn posture_refuses_start_off_loopback_without_auth() {
        // The fail-closed rule: a non-loopback bind with no admin_auth must
        // refuse to start rather than silently expose the control plane.
        for addr in ["0.0.0.0:4457", "192.168.1.10:4457", "[2001:db8::1]:4457"] {
            assert_eq!(
                server_with(addr, None).admin_auth_posture(),
                AdminAuthPosture::RefuseStart,
                "{addr} without auth must refuse to start"
            );
        }
    }

    #[test]
    fn unparseable_bind_fails_safe_to_refuse_start() {
        // An ambiguous/unparseable host is treated as non-loopback → refuse.
        assert_eq!(
            server_with("garbage-no-port", None).admin_auth_posture(),
            AdminAuthPosture::RefuseStart
        );
    }

    // ── OAuth introspect_auth fail-closed default (RFC 7662 gate) ──────────

    #[test]
    fn oauth_introspect_auth_defaults_true_via_struct_default() {
        // `..Default::default()` MUST yield introspect_auth=true — a derived
        // Default would give false and silently disable the introspection gate.
        assert!(OAuthConfig::default().introspect_auth);
        assert!(OAuthConfig { ..Default::default() }.introspect_auth);
    }

    // ── OAuth Redis-outage posture: fail-closed default + lowercase wire ──────

    #[test]
    fn oauth_backend_unavailable_defaults_to_deny() {
        // Fail-closed: an unset posture MUST be Deny on every construction path.
        assert_eq!(
            OAuthConfig::default().on_backend_unavailable,
            BackendUnavailablePosture::Deny
        );
        assert_eq!(
            BackendUnavailablePosture::default(),
            BackendUnavailablePosture::Deny
        );
    }

    #[test]
    fn oauth_backend_unavailable_missing_key_parses_deny() {
        // An `oauth: {}` block (no on_backend_unavailable key) MUST parse to Deny.
        let cfg: OAuthConfig = serde_yaml::from_str("{}").expect("empty oauth parses");
        assert_eq!(cfg.on_backend_unavailable, BackendUnavailablePosture::Deny);
    }

    #[test]
    fn oauth_backend_unavailable_serde_lowercase() {
        let cfg: OAuthConfig =
            serde_yaml::from_str("on_backend_unavailable: degrade").expect("parses degrade");
        assert_eq!(cfg.on_backend_unavailable, BackendUnavailablePosture::Degrade);
        let cfg: OAuthConfig =
            serde_yaml::from_str("on_backend_unavailable: deny").expect("parses deny");
        assert_eq!(cfg.on_backend_unavailable, BackendUnavailablePosture::Deny);
    }

    #[test]
    fn oauth_introspect_auth_defaults_true_via_serde_missing_key() {
        // An `oauth: {}` block (no introspect_auth key) MUST parse to true.
        let cfg: OAuthConfig = serde_yaml::from_str("{}").expect("empty oauth parses");
        assert!(cfg.introspect_auth);
        // And the full GateConfig default keeps it true.
        assert!(GateConfig::default().oauth.introspect_auth);
    }

    // ── https-only upstream URL validation ────────────────────────────────────

    #[test]
    fn https_upstream_always_allowed() {
        assert!(validate_upstream_url_scheme("u", "https://hydra/oauth2/token", false).is_ok());
        assert!(validate_upstream_url_scheme("u", "HTTPS://Hydra/x", false).is_ok());
    }

    #[test]
    fn http_upstream_refused_without_override() {
        let err = validate_upstream_url_scheme("hydra_token_url", "http://hydra/token", false)
            .unwrap_err();
        assert!(err.contains("plaintext"));
        assert!(err.contains("hydra_token_url"));
    }

    #[test]
    fn http_upstream_allowed_with_override() {
        assert!(validate_upstream_url_scheme("u", "http://hydra/token", true).is_ok());
    }

    #[test]
    fn non_http_scheme_refused_even_with_override() {
        // A non-http(s) scheme is never a valid credential-forwarding upstream.
        assert!(validate_upstream_url_scheme("u", "ftp://hydra/x", true).is_err());
        assert!(validate_upstream_url_scheme("u", "hydra/x", true).is_err());
    }

    // ── OAuth exposure posture ────────────────────────────────────────────────

    fn exposure_cfg(listen: &str, mount: bool, introspect_auth: bool, rl: bool) -> GateConfig {
        let mut c = GateConfig::default();
        c.server.listen = listen.to_string();
        c.oauth.introspection_enabled = mount;
        c.oauth.introspect_auth = introspect_auth;
        c.oauth.rate_limit.enabled = rl;
        c
    }

    #[test]
    fn exposure_not_mounted_when_no_oauth_capability() {
        let mut c = GateConfig::default();
        c.server.listen = "0.0.0.0:4456".into();
        c.oauth.introspection_enabled = false;
        c.oauth.client_credentials_enabled = false;
        c.token_exchange.enabled = false;
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::NotMounted);
    }

    #[test]
    fn exposure_allows_loopback_bind() {
        let c = exposure_cfg("127.0.0.1:4456", true, false, false);
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::AllowLoopback);
    }

    #[test]
    fn exposure_refuses_non_loopback_missing_introspect_auth() {
        let c = exposure_cfg("0.0.0.0:4456", true, false, true);
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::RefuseStart);
    }

    #[test]
    fn exposure_refuses_non_loopback_missing_rate_limit() {
        let c = exposure_cfg("0.0.0.0:4456", true, true, false);
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::RefuseStart);
    }

    #[test]
    fn exposure_enforces_non_loopback_with_both_guards() {
        let c = exposure_cfg("0.0.0.0:4456", true, true, true);
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::Enforce);
    }

    #[test]
    fn exposure_introspect_auth_irrelevant_when_introspection_not_mounted() {
        // Only token_exchange mounted (no introspection endpoint) → introspect_auth
        // does not gate; rate-limiting alone suffices for Enforce.
        let mut c = GateConfig::default();
        c.server.listen = "0.0.0.0:4456".into();
        c.oauth.introspection_enabled = false;
        c.token_exchange.enabled = true;
        c.oauth.rate_limit.enabled = true;
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::Enforce);
    }
}
