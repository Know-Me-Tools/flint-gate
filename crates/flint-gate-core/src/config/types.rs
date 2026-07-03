/// Configuration types for Flint Gate.
///
/// All YAML config fields map to these Rust types via serde.
/// Use `#[serde(default)]` liberally for optional fields.
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
}

/// HTTP server bind configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Proxy server listen address (default: `0.0.0.0:4456`).
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Admin API listen address (default: `0.0.0.0:4457`).
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
    "0.0.0.0:4457".to_string()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.listen, "0.0.0.0:4456");
        assert_eq!(cfg.admin_listen, "0.0.0.0:4457");
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
}
