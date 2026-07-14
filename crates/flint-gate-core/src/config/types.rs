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
    /// Per-agent tool-scope sugar. Each entry compiles to Cedar `permit`/`forbid`
    /// policies on `Action::"call_tool"` for the `Agent::"<agent>"` principal — an
    /// ergonomic front-end over the policy the engine already runs (validated at
    /// load, never a second authority). Empty by default.
    #[serde(default)]
    pub agent_tool_policies: Vec<AgentToolPolicy>,
    /// Human-in-the-loop tool-call approval (`RequireApproval`) settings.
    #[serde(default)]
    pub approval: ApprovalConfig,
}

/// Human-in-the-loop approval settings.
///
/// When a Cedar policy evaluates to `RequireApproval`, a streaming tool call is
/// paused until a human decides (via the admin API / UI). This governs the TTL of
/// that pause and whether the feature is enabled at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    /// Whether human-in-the-loop approval is enabled. When **false**, a
    /// `RequireApproval` decision **fails closed to Deny** — the tool call is NOT
    /// paused (an operator who cannot service approvals denies rather than hangs).
    /// Defaults to true (behavior unchanged).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// TTL (seconds) for a pending approval before it auto-**denies**. Overrides
    /// the built-in default (300s). An undecided approval that reaches this
    /// deadline is denied (fail-closed) and the paused stream resumes to
    /// termination — it never hangs forever.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// Maximum number of concurrent pending approvals. When the table is full,
    /// new `RequireApproval` decisions fail-closed to Deny rather than growing
    /// the in-memory DashMap without bound. Defaults to 1 000.
    #[serde(default = "default_approval_max_pending")]
    pub max_pending: Option<usize>,
    /// Override the background janitor interval (seconds). The janitor sweeps
    /// expired entries from the DashMap. When `None`, the interval is derived
    /// from `ttl_seconds / 2` (clamped to [10, 300]); a built-in fallback of
    /// 60 s applies when no TTL is set. Set this to tune the sweep frequency
    /// independently of the TTL.
    #[serde(default)]
    pub janitor_interval_seconds: Option<u64>,
}

fn default_approval_max_pending() -> Option<usize> {
    Some(1000)
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_seconds: None,
            max_pending: default_approval_max_pending(),
            janitor_interval_seconds: None,
        }
    }
}

/// Per-agent tool-scope sugar (`agent_tool_policies` entry).
///
/// Compiles to Cedar: each `allow` tool → a `permit`, each `deny` tool → a
/// `forbid` (which, per Cedar semantics, **overrides** any `permit`), all scoped
/// to `principal == Agent::"<agent>"` and `action == Action::"call_tool"`. A
/// value containing `*` is a glob matched against the tool name
/// (`context.tool_name like "<glob>"`); otherwise it is an exact
/// `resource == Route::"<tool>"` match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolPolicy {
    /// The agent principal id — becomes `Agent::"<agent>"`.
    pub agent: String,
    /// Tool names (or `*`-globs) this agent MAY call.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Tool names (or `*`-globs) this agent MUST NOT call. `deny` wins over
    /// `allow` (Cedar `forbid` overrides `permit`).
    #[serde(default)]
    pub deny: Vec<String>,
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
        // Strict cross-replica mode: when the operator demands a shared limiter
        // (`oauth.rate_limit.require_shared_backend`) but none is configured, the
        // per-replica governor cannot deliver the cross-replica ceiling they asked
        // for — refuse rather than silently under-enforce.
        let shared_backend_ok = !self.oauth.rate_limit.require_shared_backend
            || self.has_shared_ratelimit_backend();
        if introspect_guarded && rate_limited && shared_backend_ok {
            OAuthExposurePosture::Enforce
        } else {
            OAuthExposurePosture::RefuseStart
        }
    }

    /// Whether a **shared, cross-replica** rate-limit backend is actually
    /// available: an enabled L2 cache with a Redis URL **and** the `redis-l2`
    /// build feature that provides the live limiter. Both are required — a config
    /// that names Redis in a binary compiled *without* `redis-l2` has no shared
    /// limiter at runtime, so this returns `false` there and strict mode
    /// (`require_shared_backend`) genuinely refuses to start rather than falsely
    /// reporting a cross-replica limit that does not exist.
    pub fn has_shared_ratelimit_backend(&self) -> bool {
        cfg!(feature = "redis-l2")
            && self.cache.l2.enabled
            && self.cache.l2.redis_url.is_some()
    }

    /// Lint the config for **under-governed agent surfaces** — operator
    /// misconfigurations that leave an agent principal insufficiently governed.
    /// Pure and side-effect free so it is unit-testable; `main.rs` decides WARN
    /// vs refuse-start (`server.strict_agent_governance`).
    ///
    /// A route is **agent-reachable** iff its effective auth provider
    /// (`route.auth`, else the site's `default_auth`) is a **JWKS-backed** type
    /// (`jwt` or `mcp`) — those can carry an RFC 8693 `act`/agent token. Kratos is
    /// human, `api_key` is a Service credential, `anonymous` carries no principal.
    /// For each agent-reachable enabled route it flags:
    /// - a `MaxTokenBudget` left at a non-`agent` scope (agent spend accounted in
    ///   the user/team keyspace);
    /// - no `Authorize` hook at all (tool calls ungoverned by policy);
    /// - any `MaxTokenBudget` with `scope: agent` + `window: lifetime` (cannot fail
    ///   closed on a backend outage).
    ///
    /// NOTE: this convenience wrapper lints the **YAML** route set
    /// (`self.routes`). The gateway itself lints the **merged (YAML + database)**
    /// route set via [`Self::agent_governance_lint_routes`] on the
    /// [`crate::proxy::merge_routes`] output — at startup (`main.rs` step 8b) and
    /// on every route hot-reload (`rebuild_router_from_db`) — so DB-sourced and
    /// hot-reloaded routes are covered. Use this wrapper only when YAML-only
    /// linting is what you want.
    pub fn agent_governance_lint(&self) -> Vec<GovernanceFinding> {
        self.agent_governance_lint_routes(&self.routes)
    }

    /// Lint an explicit route set against this config's sites + providers. Pure;
    /// deduplicates findings per `(route_id, reason)`.
    pub fn agent_governance_lint_routes(
        &self,
        routes: &[RouteConfig],
    ) -> Vec<GovernanceFinding> {
        use std::collections::{HashMap, HashSet};
        let site_default: HashMap<&str, Option<&str>> = self
            .sites
            .iter()
            .map(|s| (s.id.as_str(), s.default_auth.as_deref()))
            .collect();

        let mut findings = Vec::new();
        let mut seen: HashSet<(String, GovernanceReason)> = HashSet::new();
        let mut push = |findings: &mut Vec<GovernanceFinding>,
                        route_id: &str,
                        reason: GovernanceReason| {
            if seen.insert((route_id.to_string(), reason)) {
                findings.push(GovernanceFinding {
                    route_id: route_id.to_string(),
                    reason,
                });
            }
        };

        for route in routes.iter().filter(|r| r.enabled) {
            // Effective provider name: route.auth, else the site's default_auth.
            let provider_name = route
                .auth
                .as_deref()
                .or_else(|| site_default.get(route.site.as_str()).copied().flatten());
            // A named-but-undefined provider is a config error (the route 500s).
            if let Some(name) = provider_name {
                if !self.auth_providers.contains_key(name) {
                    push(
                        &mut findings,
                        &route.id,
                        GovernanceReason::UnresolvableAuthProvider,
                    );
                }
            }
            let agent_reachable = provider_name
                .and_then(|n| self.auth_providers.get(n))
                .is_some_and(|p| {
                    matches!(
                        p,
                        AuthProviderConfig::Jwt(_) | AuthProviderConfig::Mcp(_)
                    )
                });

            // The Lifetime+Agent finding is independent of reachability — it is
            // always un-fail-closeable — but the other two only matter when the
            // route can actually be reached by an agent.
            let mut has_authorize = false;
            for hook in &route.hooks.pre_request {
                match hook {
                    PreRequestHook::MaxTokenBudget { config } => {
                        if config.scope == BudgetScope::Agent
                            && config.window == BudgetWindow::Lifetime
                        {
                            push(
                                &mut findings,
                                &route.id,
                                GovernanceReason::LifetimeAgentBudget,
                            );
                        }
                        if agent_reachable && config.scope != BudgetScope::Agent {
                            push(
                                &mut findings,
                                &route.id,
                                GovernanceReason::NonAgentScopedBudget,
                            );
                        }
                    }
                    PreRequestHook::Authorize { .. } => has_authorize = true,
                    _ => {}
                }
            }
            if agent_reachable && !has_authorize {
                push(&mut findings, &route.id, GovernanceReason::NoAuthorizeHook);
            }
        }
        findings
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
    /// In-process per-replica rate limiter for the **admin** router's protected
    /// endpoints. When `Some`, the governor is layered over the protected
    /// sub-router (all routes except `/health`, `/ready`, `/metrics`); probes
    /// stay unrestricted. When `None` (the default), the admin router is
    /// unlimited — suitable for loopback-dev deployments. For production
    /// non-loopback admin binds, setting this is strongly recommended.
    ///
    /// **Per-replica only.** `require_shared_backend` in [`RateLimitConfig`]
    /// is ignored here (it applies to `oauth.rate_limit` only).
    #[serde(default)]
    pub admin_rate_limit: Option<RateLimitConfig>,
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
    /// Promote agent-governance lint findings from a startup WARN to a
    /// **refuse-to-start**. On by default (fail-closed for beta deployments);
    /// set false only for legacy routes that cannot yet add an authorization hook.
    /// Any agent-reachable route without an `authorize` hook will produce a hard
    /// startup error when this is `true`. See [`GateConfig::agent_governance_lint`].
    #[serde(default = "default_strict_agent_governance")]
    pub strict_agent_governance: bool,
    /// Refuse to start when the initial Cedar policy set is empty (no policies
    /// loaded from the database or config). Disabled by default: the empty
    /// set is a valid fail-closed (default-deny) posture. Enable to enforce
    /// a non-empty policy set at startup — useful when the auth proxy MUST
    /// have explicit policies before accepting traffic.
    ///
    /// Has no effect when no database is configured (config-only deployments
    /// do not query the DB at startup so the count is always the sugar count).
    #[serde(default)]
    pub require_policies_at_startup: bool,
}

/// One agent-governance lint finding — an operator misconfiguration that leaves
/// an agent surface under-governed. Reported at startup (WARN by default, or a
/// refuse-to-start under `server.strict_agent_governance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceFinding {
    /// The route the finding is about.
    pub route_id: String,
    /// A stable, human-readable reason.
    pub reason: GovernanceReason,
}

/// The kind of agent-governance gap a [`GovernanceFinding`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GovernanceReason {
    /// An agent-reachable route has a `MaxTokenBudget` left at a non-agent scope
    /// (agent spend is accounted in the user/team keyspace, not the agent's).
    NonAgentScopedBudget,
    /// An agent-reachable route has no `Authorize` hook — tool calls are
    /// ungoverned by policy.
    NoAuthorizeHook,
    /// A budget is `scope: agent` + `window: lifetime`, which cannot fail closed
    /// on a backend outage (fail-closed agent budgets require a fixed window).
    LifetimeAgentBudget,
    /// A route names an auth provider that is not defined in `auth_providers`
    /// (typo / deleted provider) — the route 500s at runtime and can't be
    /// classified for governance.
    UnresolvableAuthProvider,
}

impl GovernanceReason {
    /// A stable one-line description for logs / the strict-mode error.
    pub fn as_str(&self) -> &'static str {
        match self {
            GovernanceReason::NonAgentScopedBudget => {
                "agent-reachable route has a non-agent-scoped token budget \
                 (agent spend is not counted under the Agent scope)"
            }
            GovernanceReason::NoAuthorizeHook => {
                "agent-reachable route has no Authorize hook (tool calls are \
                 ungoverned by policy)"
            }
            GovernanceReason::LifetimeAgentBudget => {
                "scope: agent + window: lifetime cannot fail closed on a backend \
                 outage (use a fixed window)"
            }
            GovernanceReason::UnresolvableAuthProvider => {
                "route names an auth provider that is not defined in \
                 auth_providers (typo or deleted provider) — the route will 500"
            }
        }
    }
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
    /// **OAuth strict mode (off by default).** Only meaningful on
    /// `oauth.rate_limit`. When true, the gateway refuses to start if the OAuth
    /// surface is exposed on a non-loopback bind but no **shared** (cross-replica)
    /// Redis limiter is configured (`cache.l2.enabled` + `cache.l2.redis_url`) —
    /// without one, each replica limits independently and the effective ceiling
    /// scales with the replica count. Turns "I need cross-replica-accurate limits"
    /// into an enforced invariant. Ignored on `server.rate_limit` (the in-process
    /// burst shield is per-replica by design).
    #[serde(default)]
    pub require_shared_backend: bool,
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
            require_shared_backend: false,
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
            admin_rate_limit: None,
            admin_auth: None,
            allow_insecure_upstream: false,
            strict_agent_governance: true,
            require_policies_at_startup: false,
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
    /// When false (default), a cert load failure aborts startup.
    /// Set to true only in development; never in production.
    #[serde(default)]
    pub fail_open: bool,
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

fn default_strict_agent_governance() -> bool {
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
    /// Per-agent accounting — a non-human (delegated) principal is budgeted
    /// independently of the human `User` it may act for, so agent spend is a
    /// first-class governance control (see `Identity::derived_kind`).
    Agent,
}

impl BudgetScope {
    /// A short, stable string tag used in Redis keys. Distinct per scope so the
    /// counters never collide (`flint:budget:agent:…` vs `…:user:…`).
    pub fn tag(&self) -> &'static str {
        match self {
            BudgetScope::User => "user",
            BudgetScope::Team => "team",
            BudgetScope::Agent => "agent",
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
        for (scope, tag) in [
            (BudgetScope::User, "user"),
            (BudgetScope::Team, "team"),
            (BudgetScope::Agent, "agent"),
        ] {
            assert_eq!(scope.tag(), tag);
            let json = serde_json::to_string(&scope).unwrap();
            assert_eq!(json, format!("\"{tag}\""));
            let back: BudgetScope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
        }
    }

    #[test]
    fn budget_scope_tags_are_distinct_no_key_collision() {
        use std::collections::HashSet;
        let tags: HashSet<_> = [BudgetScope::User, BudgetScope::Team, BudgetScope::Agent]
            .iter()
            .map(|s| s.tag())
            .collect();
        assert_eq!(tags.len(), 3, "each scope must key into a distinct namespace");
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

    // ── require_shared_backend strict cross-replica mode ─────────────────────

    #[test]
    fn require_shared_backend_defaults_false() {
        assert!(!RateLimitConfig::default().require_shared_backend);
        assert!(!GateConfig::default().oauth.rate_limit.require_shared_backend);
    }

    #[test]
    fn strict_shared_backend_refuses_non_loopback_without_shared_limiter() {
        // Exposed, both base guards satisfied, but require_shared_backend is set
        // and no L2/Redis limiter is configured → refuse to start.
        let mut c = exposure_cfg("0.0.0.0:4456", true, true, true);
        c.oauth.rate_limit.require_shared_backend = true;
        assert!(!c.has_shared_ratelimit_backend());
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::RefuseStart);
    }

    #[cfg(feature = "redis-l2")]
    #[test]
    fn strict_shared_backend_enforces_when_shared_limiter_configured() {
        // With the redis-l2 feature, a configured L2/Redis backend satisfies the
        // strict requirement → Enforce.
        let mut c = exposure_cfg("0.0.0.0:4456", true, true, true);
        c.oauth.rate_limit.require_shared_backend = true;
        c.cache.l2.enabled = true;
        c.cache.l2.redis_url = Some("redis://localhost:6379".into());
        assert!(c.has_shared_ratelimit_backend());
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::Enforce);
    }

    #[cfg(not(feature = "redis-l2"))]
    #[test]
    fn strict_shared_backend_refuses_without_redis_l2_feature_even_if_configured() {
        // Without the redis-l2 feature there is NO live shared limiter, so even a
        // fully-configured L2/Redis block cannot satisfy the requirement → refuse.
        let mut c = exposure_cfg("0.0.0.0:4456", true, true, true);
        c.oauth.rate_limit.require_shared_backend = true;
        c.cache.l2.enabled = true;
        c.cache.l2.redis_url = Some("redis://localhost:6379".into());
        assert!(!c.has_shared_ratelimit_backend());
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::RefuseStart);
    }

    #[test]
    fn strict_shared_backend_ignored_on_loopback() {
        // On a loopback bind the exposure guardrails (incl. this one) are not
        // enforced — local dev must still start.
        let mut c = exposure_cfg("127.0.0.1:4456", true, true, true);
        c.oauth.rate_limit.require_shared_backend = true;
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::AllowLoopback);
    }

    #[test]
    fn non_strict_starts_non_loopback_without_shared_limiter() {
        // require_shared_backend off (default) → the per-replica governor is
        // acceptable; both base guards present → Enforce (no shared limiter needed).
        let c = exposure_cfg("0.0.0.0:4456", true, true, true);
        assert!(!c.oauth.rate_limit.require_shared_backend);
        assert_eq!(c.oauth_exposure_posture(), OAuthExposurePosture::Enforce);
    }

    #[test]
    fn has_shared_ratelimit_backend_needs_both_enabled_and_url() {
        let mut c = GateConfig::default();
        assert!(!c.has_shared_ratelimit_backend());
        c.cache.l2.enabled = true;
        assert!(!c.has_shared_ratelimit_backend()); // enabled but no URL
        c.cache.l2.redis_url = Some("redis://localhost:6379".into());
        // The fully-configured case additionally requires the redis-l2 feature
        // (the live limiter); the predicate honors that so strict mode cannot be
        // satisfied by config alone in a feature-less build.
        assert_eq!(c.has_shared_ratelimit_backend(), cfg!(feature = "redis-l2"));
        c.cache.l2.enabled = false;
        assert!(!c.has_shared_ratelimit_backend()); // URL but disabled
    }

    #[test]
    fn require_shared_backend_parses_from_yaml() {
        let cfg: RateLimitConfig =
            serde_yaml::from_str("enabled: true\nrequire_shared_backend: true")
                .expect("parses");
        assert!(cfg.require_shared_backend);
        // Absent key → false (non-breaking default).
        let d: RateLimitConfig = serde_yaml::from_str("enabled: true").expect("parses");
        assert!(!d.require_shared_backend);
    }

    // ── approval config ──────────────────────────────────────────────────────

    #[test]
    fn approval_config_defaults_enabled_no_ttl() {
        let a = ApprovalConfig::default();
        assert!(a.enabled);
        assert_eq!(a.ttl_seconds, None);
        // And on a GateConfig with no approval block.
        let cfg = GateConfig::default();
        assert!(cfg.approval.enabled);
        assert_eq!(cfg.approval.ttl_seconds, None);
    }

    #[test]
    fn approval_config_parses_from_yaml() {
        let cfg: GateConfig =
            serde_yaml::from_str("approval:\n  enabled: false\n  ttl_seconds: 120\n")
                .expect("parses");
        assert!(!cfg.approval.enabled);
        assert_eq!(cfg.approval.ttl_seconds, Some(120));
    }

    #[test]
    fn approval_config_absent_key_defaults_enabled() {
        // An `approval: {}` block (no keys) must default to enabled with no TTL.
        let cfg: GateConfig = serde_yaml::from_str("approval: {}\n").expect("parses");
        assert!(cfg.approval.enabled);
        assert_eq!(cfg.approval.ttl_seconds, None);
    }

    // ── agent_tool_policies sugar (config schema) ────────────────────────────

    #[test]
    fn agent_tool_policies_default_empty() {
        assert!(GateConfig::default().agent_tool_policies.is_empty());
    }

    #[test]
    fn agent_tool_policies_parse_from_yaml() {
        let cfg: GateConfig = serde_yaml::from_str(
            "agent_tool_policies:\n  - agent: ci-bot\n    allow: [deploy, run_tests]\n    deny: [\"delete_*\"]\n",
        )
        .expect("parses");
        assert_eq!(cfg.agent_tool_policies.len(), 1);
        let p = &cfg.agent_tool_policies[0];
        assert_eq!(p.agent, "ci-bot");
        assert_eq!(p.allow, vec!["deploy", "run_tests"]);
        assert_eq!(p.deny, vec!["delete_*"]);
    }

    #[test]
    fn agent_tool_policies_allow_and_deny_default_empty() {
        // An entry may omit allow or deny — both default to empty.
        let cfg: GateConfig =
            serde_yaml::from_str("agent_tool_policies:\n  - agent: reader\n    allow: [read]\n")
                .expect("parses");
        assert_eq!(cfg.agent_tool_policies[0].allow, vec!["read"]);
        assert!(cfg.agent_tool_policies[0].deny.is_empty());
    }

    // ── agent_governance_lint ────────────────────────────────────────────────

    /// Build a GateConfig from YAML: one site + one route with the given auth
    /// provider type and pre_request hooks. `provider` is a serde tag
    /// (jwt|mcp|kratos|api_key|anonymous). `hooks_yaml` is the route's
    /// `pre_request` list (may be empty).
    fn lint_cfg(provider: &str, hooks_yaml: &str) -> GateConfig {
        let provider_body = match provider {
            "jwt" => "jwks_url: \"https://idp/jwks\"\n    issuer: \"https://idp\"",
            "mcp" => {
                "jwks_url: \"https://idp/jwks\"\n    issuer: \"https://idp\"\n    resource: \"https://rs/mcp\""
            }
            "kratos" => "base_url: \"http://kratos:4433\"",
            "api_key" => "header: \"X-API-Key\"",
            "anonymous" => "default_subject: \"anon\"",
            _ => unreachable!(),
        };
        let yaml = format!(
            "auth_providers:\n  p:\n    type: {provider}\n    {provider_body}\n\
             sites:\n  - id: s\n    default_auth: p\n\
             routes:\n  - id: r\n    site: s\n    match:\n      path: /x\n    auth: p\n\
             {hooks_yaml}\n"
        );
        serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("bad fixture: {e}\n{yaml}"))
    }

    const BUDGET_USER: &str = "    hooks:\n      pre_request:\n        - type: max_token_budget\n          config: { limit: 100, window: hour, scope: user }";
    const BUDGET_AGENT: &str = "    hooks:\n      pre_request:\n        - type: max_token_budget\n          config: { limit: 100, window: hour, scope: agent }\n        - type: authorize\n          config: {}";
    const AUTHORIZE_ONLY: &str = "    hooks:\n      pre_request:\n        - type: authorize\n          config: {}";

    fn reasons(c: &GateConfig) -> Vec<GovernanceReason> {
        c.agent_governance_lint().into_iter().map(|f| f.reason).collect()
    }

    #[test]
    fn lint_flags_agent_reachable_route_with_non_agent_budget() {
        let c = lint_cfg("jwt", BUDGET_USER);
        // jwt route with a user-scoped budget AND no Authorize -> two findings.
        let r = reasons(&c);
        assert!(r.contains(&GovernanceReason::NonAgentScopedBudget));
        assert!(r.contains(&GovernanceReason::NoAuthorizeHook));
    }

    #[test]
    fn lint_flags_agent_reachable_route_with_no_authorize_hook() {
        // mcp route with only a budget (user) -> no-authorize finding present.
        let c = lint_cfg("mcp", BUDGET_USER);
        assert!(reasons(&c).contains(&GovernanceReason::NoAuthorizeHook));
    }

    #[test]
    fn lint_clean_agent_route_has_no_findings() {
        // jwt route with an AGENT-scoped budget + an Authorize hook -> clean.
        let c = lint_cfg("jwt", BUDGET_AGENT);
        assert!(c.agent_governance_lint().is_empty(), "{:?}", reasons(&c));
    }

    #[test]
    fn lint_ignores_non_agent_reachable_providers() {
        // Kratos / api_key / anonymous routes are NOT agent-reachable — even a
        // user-scoped budget with no Authorize yields nothing.
        for provider in ["kratos", "api_key", "anonymous"] {
            let c = lint_cfg(provider, BUDGET_USER);
            assert!(
                c.agent_governance_lint().is_empty(),
                "{provider} must not be agent-reachable: {:?}",
                reasons(&c)
            );
        }
    }

    #[test]
    fn lint_flags_lifetime_agent_budget_regardless_of_reachability() {
        // scope: agent + window: lifetime is un-fail-closeable — flagged even on a
        // non-agent-reachable (kratos) route.
        let hooks = "    hooks:\n      pre_request:\n        - type: max_token_budget\n          config: { limit: 100, window: lifetime, scope: agent }";
        let c = lint_cfg("kratos", hooks);
        assert!(reasons(&c).contains(&GovernanceReason::LifetimeAgentBudget));
    }

    #[test]
    fn lint_agent_route_with_only_authorize_is_clean() {
        // An agent-reachable route with an Authorize hook and no budget is fine
        // (a budget is optional; the lint only flags a MIS-scoped one).
        let c = lint_cfg("jwt", AUTHORIZE_ONLY);
        assert!(c.agent_governance_lint().is_empty(), "{:?}", reasons(&c));
    }

    #[test]
    fn strict_agent_governance_defaults_true() {
        assert!(ServerConfig::default().strict_agent_governance);
    }

    #[test]
    fn lint_flags_unresolvable_auth_provider() {
        // A route naming a provider that isn't defined -> UnresolvableAuthProvider.
        let yaml = "auth_providers:\n  p:\n    type: jwt\n    jwks_url: \"https://idp/jwks\"\n    issuer: \"https://idp\"\n\
                    sites:\n  - id: s\n    default_auth: p\n\
                    routes:\n  - id: r\n    site: s\n    match:\n      path: /x\n    auth: typo-provider\n";
        let c: GateConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(reasons(&c).contains(&GovernanceReason::UnresolvableAuthProvider));
    }

    #[test]
    fn lint_deduplicates_findings_per_route_and_reason() {
        // Two non-agent budgets on one agent-reachable route -> a SINGLE
        // NonAgentScopedBudget finding (deduped), not two.
        let hooks = "    hooks:\n      pre_request:\n\
                     \x20       - type: max_token_budget\n          config: { limit: 100, window: hour, scope: user }\n\
                     \x20       - type: max_token_budget\n          config: { limit: 200, window: day, scope: user }";
        let c = lint_cfg("jwt", hooks);
        let n = c
            .agent_governance_lint()
            .iter()
            .filter(|f| f.reason == GovernanceReason::NonAgentScopedBudget)
            .count();
        assert_eq!(n, 1, "duplicate non-agent-budget findings must be deduped");
    }

    /// task 4: strict mode on + agent-reachable route without authorize hook → findings non-empty.
    /// The startup path converts non-empty findings to a hard error when strict=true.
    #[test]
    fn strict_mode_on_with_ungoverned_agent_route_produces_findings() {
        // jwt route with no hooks at all — no authorize, no budget
        let c = lint_cfg("jwt", "");
        // Strict mode on (the new default)
        let findings = c.agent_governance_lint();
        assert!(
            !findings.is_empty(),
            "agent-reachable route with no authorize hook must produce governance findings"
        );
        assert!(
            findings.iter().any(|f| f.reason == GovernanceReason::NoAuthorizeHook),
            "missing authorize hook must surface as NoAuthorizeHook finding"
        );
        // Confirm: with strict=true and findings, startup should refuse.
        // (The actual bail! is in main.rs; here we assert the condition is met.)
        assert!(c.server.strict_agent_governance, "strict mode should default to true");
    }

    /// task 5: strict mode on + agent-reachable route WITH authorize hook → no findings.
    #[test]
    fn strict_mode_on_with_governed_agent_route_produces_no_findings() {
        let c = lint_cfg("jwt", AUTHORIZE_ONLY);
        let findings = c.agent_governance_lint();
        assert!(
            findings.is_empty(),
            "agent-reachable route WITH authorize hook must produce no governance findings: {:?}",
            findings
        );
        assert!(c.server.strict_agent_governance, "strict mode should default to true");
    }

    #[test]
    fn lint_routes_helper_lints_an_explicit_route_set() {
        // agent_governance_lint_routes lets a caller lint DB-sourced routes too.
        let base = lint_cfg("jwt", BUDGET_AGENT); // clean YAML route set
        assert!(base.agent_governance_lint().is_empty());
        // A separately-supplied (e.g. DB) route with a user budget is flagged.
        let mut extra = base.routes[0].clone();
        extra.id = "db-route".into();
        extra.hooks.pre_request = vec![PreRequestHook::MaxTokenBudget {
            config: MaxTokenBudgetConfig {
                limit: 10,
                user_id_expr: "identity.id".into(),
                error_message: None,
                window: BudgetWindow::Hour,
                scope: BudgetScope::User,
            },
        }];
        let f = base.agent_governance_lint_routes(&[extra]);
        assert!(f.iter().any(|x| x.route_id == "db-route"
            && x.reason == GovernanceReason::NonAgentScopedBudget));
    }
}
