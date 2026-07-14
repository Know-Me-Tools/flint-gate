//! Flint Gate — AI-native auth proxy and API gateway.
//!
//! Configuration priority (highest → lowest):
//!   CLI flags  >  environment variables  >  config.yaml

use flint_gate_core::admin::{AdminEvent, AdminState};
use flint_gate_core::auth::{build_authenticators, JwtMinter, SharedJwtMinter};
use flint_gate_core::authz::AuthzEngine;
use flint_gate_core::cache::{start_cache_invalidation_listener, GateCache};
use flint_gate_core::config::{load_config, GateConfig, LookupRegistry};
use flint_gate_core::db::Database;
use flint_gate_core::middleware::{proxy_handler, AppState};
use flint_gate_core::proxy::{Router as GateRouter, SharedRouter};

use anyhow::{Context, Result};
use axum::{
    routing::{any, get, post},
    Json, Router,
};
use clap::Parser;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Flint Gate — AI-native auth proxy and API gateway.
///
/// Configuration is resolved in priority order (highest first):
///   1. CLI flags   (--listen 0.0.0.0:8080)
///   2. Environment (FLINT_GATE_LISTEN=0.0.0.0:8080)
///   3. YAML file   (server.listen: "0.0.0.0:8080")
#[derive(Parser, Debug, Clone)]
#[command(
    name = "flint-gate",
    version,
    about = "AI-native auth proxy and API gateway"
)]
struct Cli {
    /// Path to the YAML configuration file.
    #[arg(
        short = 'c',
        long,
        env = "FLINT_GATE_CONFIG",
        default_value = "config.yaml",
        value_name = "PATH"
    )]
    config: String,

    /// Proxy server listen address. Overrides server.listen in config.yaml.
    #[arg(long, env = "FLINT_GATE_LISTEN", value_name = "HOST:PORT")]
    listen: Option<String>,

    /// Admin API listen address. Overrides server.admin_listen in config.yaml.
    #[arg(long, env = "FLINT_GATE_ADMIN_LISTEN", value_name = "HOST:PORT")]
    admin_listen: Option<String>,

    /// Postgres connection URL. Overrides database.url in config.yaml.
    #[arg(long, env = "DATABASE_URL", value_name = "URL")]
    database_url: Option<String>,

    /// Tracing filter (EnvFilter syntax). E.g. `debug`, `info,flint_gate=trace`.
    #[arg(
        long,
        env = "RUST_LOG",
        default_value = "info,flint_gate=debug",
        value_name = "FILTER"
    )]
    log: String,

    /// HMAC secret for HS256 JWT signing. Overrides jwt.signing_key_secret.
    #[arg(long, env = "FLINT_GATE_JWT_SECRET", value_name = "SECRET")]
    jwt_secret: Option<String>,

    /// Path to PEM private key for RS256/ES256 JWT signing. Overrides jwt.signing_key_path.
    #[arg(long, env = "FLINT_GATE_JWT_KEY_PATH", value_name = "PATH")]
    jwt_key_path: Option<String>,
}

/// Apply CLI / env-var overrides onto a loaded [`GateConfig`].
///
/// Invoked at startup and after every YAML hot-reload so that CLI-supplied
/// values always win over what is on disk.
fn apply_overrides(mut cfg: GateConfig, cli: &Cli) -> GateConfig {
    if let Some(v) = &cli.listen {
        cfg.server.listen = v.clone();
    }
    if let Some(v) = &cli.admin_listen {
        cfg.server.admin_listen = v.clone();
    }
    if let Some(v) = &cli.database_url {
        cfg.database.url = v.clone();
    }
    if let Some(v) = &cli.jwt_secret {
        cfg.jwt.signing_key_secret = Some(v.clone());
    }
    if let Some(v) = &cli.jwt_key_path {
        cfg.jwt.signing_key_path = Some(v.clone());
    }
    cfg
}

// ── Shutdown signal ───────────────────────────────────────────────────────────

/// Wait for SIGTERM (Unix) or SIGINT (Ctrl-C) — whichever arrives first.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigterm.recv() => info!("received SIGTERM"),
            _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
        info!("received SIGINT");
    }
}

// ── TLS config guard ──────────────────────────────────────────────────────────

/// Returns `Ok(())` when the TLS config is coherent, or `Err` when `fail_open`
/// is false and the config is misconfigured (no cert/key paths supplied).
/// The actual cert load failure path is handled inline in the startup block;
/// this covers the "TLS enabled but paths not set" case at config-validation time.
fn check_tls_config(tls: &flint_gate_core::config::types::TlsConfig) -> Result<()> {
    if tls.enabled && (tls.cert_path.is_none() || tls.key_path.is_none()) && !tls.fail_open {
        anyhow::bail!(
            "TLS is enabled but tls.cert_path/tls.key_path are not configured \
             and tls.fail_open is false — refusing to start"
        );
    }
    Ok(())
}

/// Returns `true` when flint-gate is running in Kubernetes without admin_auth —
/// the condition that triggers the unauthenticated-admin-in-k8s warning.
fn k8s_admin_unprotected(admin_auth_configured: bool) -> bool {
    std::env::var("KUBERNETES_SERVICE_HOST").is_ok() && !admin_auth_configured
}

/// Returns the replica count from `REPLICA_COUNT` env var if set and parseable,
/// and whether it exceeds 1 — the condition that triggers the sticky-session warning.
fn multi_replica_count() -> Option<usize> {
    std::env::var("REPLICA_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
}

/// Returns `true` when rate limiting is enabled without Redis in a Kubernetes
/// environment — the condition where per-replica counters diverge and agents
/// can exceed the configured budget by issuing requests across replicas.
fn rate_limit_needs_redis_warning(rate_limit_enabled: bool, redis_url_configured: bool) -> bool {
    rate_limit_enabled
        && !redis_url_configured
        && std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI (env vars auto-applied by clap)
    let cli = Cli::parse();

    // 2. Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&cli.log))
        .with_target(true)
        .init();

    info!("⚡ Flint Gate starting — Strike an idea. Watch it build.");

    // Install the Prometheus recorder (idempotent) so control-plane metrics
    // (e.g. delegate outcomes) are collected and renderable on /metrics.
    let _metrics_handle = flint_gate_core::metrics::install_recorder();

    info!(config = %cli.config, "loading config");

    // 3. Load YAML config
    let (shared_config, mut config_rx) = load_config(&cli.config)
        .await
        .with_context(|| format!("loading config from {}", cli.config))?;

    // 4. Apply CLI / env overrides on top of YAML
    {
        let mut cfg = shared_config.write().await;
        *cfg = apply_overrides(cfg.clone(), &cli);
    }
    let initial_config = shared_config.read().await.clone();
    info!(
        proxy_listen = %initial_config.server.listen,
        admin_listen = %initial_config.server.admin_listen,
        "config ready"
    );

    // 4b. Agent-governance lint runs AFTER the DB routes are loaded (step 8), so
    // it lints the MERGED (YAML + DB) route set actually served — not YAML alone.
    // See the merged-set lint just after the route table is built below.

    // 4c. Agent tool-scope sugar: compile `agent_tool_policies` to Cedar and run
    // every emitted policy through the write-time validator BEFORE serving. A
    // sugar block that compiles to invalid Cedar refuses start (fail-closed — a
    // bad policy never loads). The sugar is a validated front-end over the Cedar
    // the engine already runs, not a second authority.
    let sugar_policies = match flint_gate_core::authz::compile_and_validate(
        &initial_config.agent_tool_policies,
    ) {
        Ok(records) => records,
        Err(e) => anyhow::bail!("refusing to start: invalid agent_tool_policies — {e}"),
    };
    if !sugar_policies.is_empty() {
        info!(
            count = sugar_policies.len(),
            "compiled agent_tool_policies sugar to validated Cedar policies"
        );
    }

    // 5. Connect to database
    let db = if initial_config.database.url.is_empty() {
        info!("no database URL configured; DB features disabled");
        None
    } else {
        match Database::connect(
            &initial_config.database.url,
            initial_config.database.max_connections,
        )
        .await
        {
            Ok(d) => {
                d.migrate().await.context("applying database migrations")?;
                info!("database connected");
                Some(Arc::new(d))
            }
            Err(e) => {
                warn!(error = %e, "database connection failed; running without DB features");
                None
            }
        }
    };

    // 6. Build authenticators
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("building HTTP client")?;

    let auth_providers = Arc::new(build_authenticators(
        &initial_config.auth_providers,
        &http_client,
        db.clone(),
    ));
    info!(count = auth_providers.len(), "auth providers initialized");

    // 7. Build JWT minter (prefer DB-sourced key, fall back to config)
    let jwt_minter: SharedJwtMinter = Arc::new(RwLock::new(None));
    let jwt_cfg = &initial_config.jwt;
    if jwt_cfg.signing_key_secret.is_some() || jwt_cfg.signing_key_path.is_some() || db.is_some() {
        match JwtMinter::from_db_or_config(db.as_deref(), jwt_cfg).await {
            Ok(m) => {
                *jwt_minter.write().await = Some(m);
                info!(algorithm = %jwt_cfg.signing_algorithm, "JWT minter initialized");
            }
            Err(e) => warn!(error = %e, "JWT minter init failed; minting disabled"),
        }
    } else {
        info!("no JWT signing key configured; JWT minting disabled");
    }

    // 8. Compute the merged (YAML + DB) route set, then build the router from it.
    // Merging into an explicit Vec (rather than straight into the router) lets the
    // agent-governance lint below inspect the exact routes that will be served.
    let merged_routes: Vec<flint_gate_core::config::types::RouteConfig> =
        if initial_config.database.override_yaml {
            if let Some(ref d) = db {
                match d.load_routes().await {
                    Ok(db_routes) => {
                        info!(count = db_routes.len(), "loaded DB routes for override mode");
                        flint_gate_core::proxy::merge_routes(&initial_config, &db_routes)
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to load DB routes — falling back to YAML only");
                        initial_config.routes.clone()
                    }
                }
            } else {
                initial_config.routes.clone()
            }
        } else {
            initial_config.routes.clone()
        };

    // 8b. Agent-governance lint over the MERGED route set (YAML + DB), so a
    // DB-only under-governed agent route is surfaced too. WARN by default; refuse
    // to start under `server.strict_agent_governance` (safe — pre-serve). Linting
    // the merged set once avoids double-WARNing YAML routes.
    let governance_findings = initial_config.agent_governance_lint_routes(&merged_routes);
    for f in &governance_findings {
        warn!(route_id = %f.route_id, "agent-governance: {}", f.reason.as_str());
    }
    if initial_config.server.strict_agent_governance && !governance_findings.is_empty() {
        let detail = governance_findings
            .iter()
            .map(|f| format!("route {}: {}", f.route_id, f.reason.as_str()))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!(
            "refusing to start: {} agent-governance finding(s) with \
             server.strict_agent_governance=true — {detail}",
            governance_findings.len()
        );
    }

    let gate_router = GateRouter::from_config_with_routes(&initial_config, merged_routes);
    info!(route_count = gate_router.route_count(), "route table built");
    let shared_router: SharedRouter = Arc::new(RwLock::new(gate_router));

    // 9. Build cache + LISTEN/NOTIFY invalidation
    // `mut` is only needed by `connect_l2` under the `redis-l2` feature.
    #[cfg_attr(not(feature = "redis-l2"), allow(unused_mut))]
    let mut cache = GateCache::from_config(&initial_config.cache);
    #[cfg(feature = "redis-l2")]
    {
        if let Err(e) = cache.connect_l2(&initial_config.cache).await {
            warn!(error = %e, "Redis L2 cache connection failed — continuing with L1 only");
        }
    }
    let cache = Arc::new(cache);

    // Build the embedded Cedar authorization engine BEFORE the LISTEN/NOTIFY
    // listener so its handle can be threaded in — a "policies" NOTIFY on any
    // replica must reload this engine (multi-replica hot-reload). The validated
    // `agent_tool_policies` sugar is carried as an IMMUTABLE OVERLAY on the engine
    // and re-applied on every reload, so config tool-scopes enforce ALONGSIDE the
    // DB policies (Cedar forbid-overrides-permit resolves cross-source conflicts).
    // With a database: initial bundle = DB rows ++ sugar overlay (lenient — bad
    // rows skipped). Without: seed from the sugar overlay alone (pure-config
    // deployment), else empty (default-deny).
    let authz = match &db {
        Some(d) => Arc::new(
            AuthzEngine::from_database_with_sugar(d, sugar_policies.clone()).await,
        ),
        None if !sugar_policies.is_empty() => Arc::new(
            AuthzEngine::from_records_with_sugar(&[], sugar_policies.clone())
                .map_err(|e| anyhow::anyhow!("failed to build authz engine from sugar: {e}"))?,
        ),
        None => Arc::new(AuthzEngine::empty()),
    };

    // Fail-closed startup gate: when require_policies_at_startup is true and
    // the loaded engine carries zero policies, refuse to start. This is only
    // checked when a DB is configured (no-DB deployments load from config sugar
    // alone; they should set sugar policies, not this flag).
    if initial_config.server.require_policies_at_startup && db.is_some() {
        let policy_count = authz.snapshot().policies().policies().count();
        if policy_count == 0 {
            tracing::error!(
                "require_policies_at_startup is true but no policies are loaded — refusing to start"
            );
            std::process::exit(1);
        }
    }

    // Admin event broadcast channel — shared between the cache invalidation
    // listener (emitter) and the admin SSE endpoint (subscriber).
    let (admin_event_tx, _admin_event_rx) = tokio::sync::broadcast::channel::<AdminEvent>(256);

    if let Some(ref d) = db {
        let ch = initial_config.cache.invalidation_channel.clone();
        // When override_yaml is enabled, pass the router + config + DB so the
        // listener can rebuild the router on every "routes" NOTIFY.
        let router_ctx = if initial_config.database.override_yaml {
            Some((
                Arc::clone(&shared_router),
                Arc::clone(&shared_config),
                Arc::clone(d),
            ))
        } else {
            None
        };
        // Thread the authz engine + DB so a "policies" NOTIFY reloads it.
        let authz_ctx = Some((Arc::clone(&authz), Arc::clone(d)));
        start_cache_invalidation_listener(
            d.pool(),
            Arc::clone(&cache),
            ch,
            router_ctx,
            authz_ctx,
            Some(admin_event_tx.clone()),
        )
        .await;
    }

    // 10. Build lookup registry
    let lookup_registry = Arc::new(LookupRegistry::new(db.clone()));

    // 10b. Build the shared Redis rate limiter by reusing the L2 connection.
    #[cfg(feature = "redis-l2")]
    let rate_limiter = cache
        .l2_connection()
        .map(flint_gate_core::ratelimit::RedisRateLimiter::new);
    #[cfg(feature = "redis-l2")]
    if rate_limiter.is_some() {
        info!("shared Redis window counters enabled (budgets + request-rate)");
    }
    // Clone the shared limiter for the OAuth sub-router (built below, after the
    // limiter is moved into AppState). `RedisRateLimiter` is cheap to clone.
    #[cfg(feature = "redis-l2")]
    let oauth_rate_limiter = rate_limiter.clone();

    // 10c. Human-in-the-loop approval routing table (in-process, per-replica).
    //      Wire the configured capacity cap so register() fails-closed on overflow.
    let approval_manager = {
        let cap = initial_config.approval.max_pending;
        match cap {
            Some(n) => {
                info!(cap = n, "approval manager: capacity cap enabled");
                flint_gate_core::approval::ApprovalManager::with_cap(n)
            }
            None => {
                warn!("approval manager: no capacity cap configured (unbounded); set approval.max_pending for production");
                flint_gate_core::approval::ApprovalManager::new()
            }
        }
    };

    // 10d. Approval janitor: periodically reap expired pending approvals so
    // entries whose streams have already ended do not accumulate in the DashMap.
    // (The paused-stream task auto-denies an undecided approval on its own TTL —
    // this is hygiene for the already-ended case.) Per-replica, like the manager.
    {
        let janitor_manager = approval_manager.clone();
        let janitor_interval = {
            // Use explicit config value when set; otherwise derive from TTL / 2
            // (clamped to [10, 300]), with 60 s as the final fallback.
            if let Some(explicit) = initial_config.approval.janitor_interval_seconds {
                explicit
            } else {
                initial_config
                    .approval
                    .ttl_seconds
                    .map(|t| (t / 2).clamp(10, 300))
                    .unwrap_or(60)
            }
        };
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(janitor_interval));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let reaped = janitor_manager.purge_expired();
                if reaped > 0 {
                    tracing::debug!(reaped, "approval janitor purged expired pending approvals");
                }
            }
        });
    }

    // 11. Assemble AppState
    let app_state = Arc::new(AppState {
        config: Arc::clone(&shared_config),
        router: Arc::clone(&shared_router),
        auth_providers: Arc::clone(&auth_providers),
        jwt_minter: Arc::clone(&jwt_minter),
        cache: Arc::clone(&cache),
        db: db.clone(),
        http_client: http_client.clone(),
        lookup_registry: Arc::clone(&lookup_registry),
        authz: Arc::clone(&authz),
        approval_manager: approval_manager.clone(),
        #[cfg(feature = "redis-l2")]
        rate_limiter,
    });

    let shutdown_timeout = initial_config.server.shutdown_timeout_secs;
    let token = CancellationToken::new();

    // 12. Start proxy server — with /health shortcut and optional TLS
    let proxy_listen = initial_config.server.listen.clone();
    // RFC 9728 Protected Resource Metadata — served on the PUBLIC proxy surface
    // (MCP clients must reach it) rather than the private admin port. Captures
    // the shared config so a hot-reload of MCP providers is reflected live.
    let metadata_config = Arc::clone(&shared_config);
    let mut proxy_app = Router::new()
        .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route(
            flint_gate_core::auth::mcp_metadata::PROTECTED_RESOURCE_METADATA_PATH,
            get(move || {
                let cfg = Arc::clone(&metadata_config);
                async move {
                    flint_gate_core::auth::mcp_metadata::protected_resource_metadata_handler(cfg)
                        .await
                }
            }),
        )
        .fallback(any(proxy_handler))
        .with_state(Arc::clone(&app_state))
        .layer(TraceLayer::new_for_http());

    // OAuth 2.0 endpoints (proxy port): unified `/oauth/token` (RFC 8693 token
    // exchange + RFC 6749 client-credentials, dispatched by grant_type) and
    // RFC 7662 `/oauth/introspect`. Each capability is independently gated.
    {
        use flint_gate_core::auth::introspect::TokenVerifier;
        use flint_gate_core::auth::oauth::{IntrospectionState, OAuthState};
        use flint_gate_core::auth::token_exchange::validate_subject_provider;
        use flint_gate_core::config::types::OAuthExposurePosture;

        // Exposure gate: refuse to start with an under-guarded /oauth/* surface
        // on a non-loopback proxy bind (fail-safe — the endpoints are then
        // internet-reachable and MUST have introspect-auth + rate-limiting).
        match initial_config.oauth_exposure_posture() {
            OAuthExposurePosture::RefuseStart => anyhow::bail!(
                "refusing to start: /oauth/* is mounted on a non-loopback bind \
                 ({}) without the required guardrails — need oauth.introspect_auth, \
                 oauth.rate_limit.enabled, and (when oauth.rate_limit.require_shared_backend \
                 is set) a shared limiter via cache.l2.enabled + cache.l2.redis_url. \
                 Enable the missing guard(s), or bind server.listen to loopback for local development.",
                initial_config.server.listen
            ),
            OAuthExposurePosture::AllowLoopback => {
                info!("OAuth surface on a loopback bind — exposure guardrails not enforced (dev)")
            }
            OAuthExposurePosture::Enforce => {
                info!("OAuth surface exposed with introspect-auth + rate-limiting (guardrails enforced)")
            }
            OAuthExposurePosture::NotMounted => {}
        }

        // Token exchange: only when enabled with a fail-closed subject provider.
        let tx_cfg = &initial_config.token_exchange;
        let token_exchange = if tx_cfg.enabled {
            let provider_name = tx_cfg.subject_token_provider.as_ref().ok_or_else(|| {
                anyhow::anyhow!("token_exchange.enabled is true but subject_token_provider is not set")
            })?;
            let provider_cfg = initial_config
                .auth_providers
                .get(provider_name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "token_exchange subject_token_provider {provider_name:?} is not a configured auth provider"
                    )
                })?;
            if let Err(reason) = validate_subject_provider(provider_cfg) {
                anyhow::bail!("refusing to enable token exchange: {reason}");
            }
            let verifier = auth_providers.get(provider_name).cloned().ok_or_else(|| {
                anyhow::anyhow!("subject_token_provider {provider_name:?} authenticator was not built")
            })?;
            // Optional Hydra-delegate: proxy the exchange to a configured Hydra
            // token endpoint (federate-first). Requires delegate_to_hydra + a URL.
            let delegate = if tx_cfg.delegate_to_hydra {
                let url = tx_cfg.hydra_token_url.clone().ok_or_else(|| {
                    anyhow::anyhow!(
                        "token_exchange.delegate_to_hydra is true but hydra_token_url is not set"
                    )
                })?;
                // https-only unless explicitly overridden — the delegate forwards
                // the subject_token, so a plaintext URL leaks it on the wire.
                flint_gate_core::config::types::validate_upstream_url_scheme(
                    "token_exchange.hydra_token_url",
                    &url,
                    initial_config.server.allow_insecure_upstream,
                )
                .map_err(|e| anyhow::anyhow!(e))?;
                if initial_config.server.allow_insecure_upstream && url.starts_with("http://") {
                    warn!(token_url = %url,
                        "INSECURE: forwarding subject_token to a plaintext http:// Hydra token endpoint (allow_insecure_upstream)");
                }
                // Dedicated client that does NOT follow redirects: the delegate
                // forwards the subject_token to a FIXED operator-configured URL,
                // so a Hydra 3xx must never be followed to another host (that
                // would exfiltrate the subject token to an attacker).
                let delegate_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .context("building Hydra-delegate HTTP client")?;
                info!(token_url = %url, "RFC 8693 token exchange enabled (Hydra delegate)");
                Some(flint_gate_core::auth::token_exchange::HydraDelegate {
                    http_client: delegate_client,
                    token_url: url,
                })
            } else {
                info!("RFC 8693 token exchange enabled (gateway-local)");
                None
            };
            Some((verifier, tx_cfg.delegated_ttl_seconds, delegate))
        } else {
            None
        };

        let oauth_cfg = &initial_config.oauth;

        // Reject a degenerate rate-limit config that would near-totally lock out
        // the OAuth surface: enabled with per_second == 0 yields a ceiling of 1
        // (2nd request/min denied). Fail fast at startup rather than silently.
        if oauth_cfg.rate_limit.enabled && oauth_cfg.rate_limit.per_second == 0 {
            anyhow::bail!(
                "oauth.rate_limit.enabled is true but per_second is 0 — this would \
                 deny after a single request per window. Set per_second >= 1 or \
                 disable oauth.rate_limit."
            );
        }

        // Client credentials: needs the DB-backed client store.
        let client_credentials = if oauth_cfg.client_credentials_enabled {
            match db.clone() {
                Some(database) => {
                    info!("OAuth client_credentials grant enabled");
                    Some((database, oauth_cfg.service_token_ttl_seconds))
                }
                None => anyhow::bail!(
                    "oauth.client_credentials_enabled is true but no database is configured (the client store lives in Postgres)"
                ),
            }
        } else {
            None
        };

        // Introspection: build a verifier from the gateway signing config.
        let introspection = if oauth_cfg.introspection_enabled {
            let verifier = TokenVerifier::from_jwt_config(&initial_config.jwt)
                .await
                .context("building introspection verifier from jwt config")?;
            // RFC 7662 §2.1: introspection auth is required by default. Refuse to
            // start if auth is required but there is no client store to verify
            // against (fail-closed — never run an unauthable-but-required endpoint).
            if oauth_cfg.introspect_auth && db.is_none() {
                anyhow::bail!(
                    "oauth.introspect_auth is true but no database is configured — \
                     the client store is required to authenticate /oauth/introspect. \
                     Configure a database or set oauth.introspect_auth=false (only when \
                     the endpoint is network-restricted)."
                );
            }
            if oauth_cfg.introspect_auth {
                info!("RFC 7662 introspection enabled (client auth REQUIRED)");
            } else {
                info!(
                    "RFC 7662 introspection enabled — UNAUTHENTICATED (introspect_auth=false); \
                     ensure /oauth/introspect is network-restricted"
                );
            }
            // https-only unless overridden — the introspection delegate proxies
            // to Hydra's ADMIN API, so a plaintext URL exposes that surface.
            if let Some(d) = oauth_cfg.introspection_delegate.as_ref() {
                flint_gate_core::config::types::validate_upstream_url_scheme(
                    "oauth.introspection_delegate.hydra_admin_url",
                    &d.hydra_admin_url,
                    initial_config.server.allow_insecure_upstream,
                )
                .map_err(|e| anyhow::anyhow!(e))?;
                if initial_config.server.allow_insecure_upstream
                    && d.hydra_admin_url.starts_with("http://")
                {
                    warn!(hydra_admin_url = %d.hydra_admin_url,
                        "INSECURE: introspection delegate targets a plaintext http:// Hydra admin endpoint (allow_insecure_upstream)");
                }
            }
            // Dedicated no-redirect client for the introspection delegate: it
            // POSTs the caller's token to Hydra's admin endpoint, so a Hydra 3xx
            // must NOT be followed to another host (token-exfiltration guard —
            // parity with the token-exchange delegate).
            let introspect_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("building introspection-delegate HTTP client")?;
            Some(IntrospectionState {
                verifier,
                http_client: introspect_client,
                hydra_admin_url: oauth_cfg
                    .introspection_delegate
                    .as_ref()
                    .map(|d| d.hydra_admin_url.clone()),
                require_auth: oauth_cfg.introspect_auth,
                client_store: db.clone(),
            })
        } else {
            None
        };

        // Mount only if at least one capability is on.
        if token_exchange.is_some() || client_credentials.is_some() || introspection.is_some() {
            let oauth_state = OAuthState {
                minter: Arc::clone(&jwt_minter),
                token_exchange,
                client_credentials,
                introspection,
                // Populated by build_oauth_routes when a shared limiter exists.
                #[cfg(feature = "redis-l2")]
                token_limiter: None,
                #[cfg(feature = "redis-l2")]
                introspect_limiter: None,
            };
            // Build the two OAuth routes separately so each can carry its own
            // shared-limiter posture (the introspection oracle always denies on a
            // Redis outage; the token endpoint may degrade). `build_oauth_routes`
            // attaches the shared Redis limiter (authoritative cross-replica) when
            // present, keyed by credential/IP.
            let orl = &oauth_cfg.rate_limit;
            let mut oauth_router = build_oauth_routes(
                oauth_state,
                orl,
                #[cfg(feature = "redis-l2")]
                oauth_rate_limiter.clone(),
                #[cfg(feature = "redis-l2")]
                oauth_cfg.on_backend_unavailable,
            );

            // In-process governor across BOTH routes: the coarse per-replica
            // burst shield AND the degrade target when a token-endpoint request
            // finds the shared backend unavailable under the `degrade` posture.
            if orl.enabled {
                match flint_gate_core::ratelimit::build_governor_layer(orl.per_second, orl.burst) {
                    Some(layer) => {
                        oauth_router = oauth_router.layer(layer);
                        info!(
                            per_second = orl.per_second,
                            burst = orl.burst,
                            "OAuth endpoint rate limiter enabled (in-process governor)"
                        );
                    }
                    None => warn!("oauth.rate_limit enabled but config degenerate — not applied"),
                }
            }

            proxy_app = proxy_app.merge(oauth_router);
        }
    }

    // Coarse in-process request-rate shield (per replica), keyed on credential
    // with client-IP fallback. Authoritative cross-replica limiting is the
    // Redis window counters; this only clips bursts.
    let rate_cfg = &initial_config.server.rate_limit;
    if rate_cfg.enabled {
        let redis_configured = initial_config.cache.l2.redis_url.is_some();
        if rate_limit_needs_redis_warning(true, redis_configured) {
            warn!(
                "rate limiting enabled without Redis in a Kubernetes environment \
                 — per-replica counters will not be shared; configure \
                 cache.l2.redis_url for correct multi-replica behavior"
            );
        }
        match flint_gate_core::ratelimit::build_governor_layer(rate_cfg.per_second, rate_cfg.burst)
        {
            Some(layer) => {
                proxy_app = proxy_app.layer(layer);
                info!(
                    per_second = rate_cfg.per_second,
                    burst = rate_cfg.burst,
                    "in-process request-rate limiter enabled"
                );
            }
            None => warn!("rate_limit enabled but config was degenerate — limiter not applied"),
        }
    }

    let tls_cfg = initial_config.server.tls.clone();
    let proxy_token = token.clone();
    let mut proxy_server = if tls_cfg.enabled {
        match (tls_cfg.cert_path.as_deref(), tls_cfg.key_path.as_deref()) {
            (Some(cert), Some(key)) => {
                match axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await {
                    Ok(rustls_config) => {
                        let addr: std::net::SocketAddr =
                            proxy_listen.parse().with_context(|| {
                                format!("invalid proxy listen address: {proxy_listen}")
                            })?;
                        let axum_handle = axum_server::Handle::new();
                        let h = axum_handle.clone();
                        tokio::spawn(async move {
                            proxy_token.cancelled().await;
                            h.graceful_shutdown(Some(Duration::from_secs(shutdown_timeout)));
                        });
                        info!(addr = %proxy_listen, "proxy server listening (TLS)");
                        tokio::spawn(async move {
                            if let Err(e) = axum_server::bind_rustls(addr, rustls_config)
                                .handle(axum_handle)
                                .serve(proxy_app.into_make_service())
                                .await
                            {
                                error!(error = %e, "proxy TLS server error");
                            }
                        })
                    }
                    Err(e) => {
                        if !tls_cfg.fail_open {
                            anyhow::bail!(
                                "failed to load TLS cert/key and tls.fail_open is false: {e}"
                            );
                        }
                        warn!(
                            error = %e,
                            "WARN: TLS fail-open enabled — falling back to plain TCP despite cert/key load failure"
                        );
                        let listener = tokio::net::TcpListener::bind(&proxy_listen)
                            .await
                            .with_context(|| format!("binding proxy server to {proxy_listen}"))?;
                        info!(addr = %proxy_listen, "proxy server listening");
                        tokio::spawn(async move {
                            if let Err(e) = axum::serve(listener, proxy_app)
                                .with_graceful_shutdown(
                                    async move { proxy_token.cancelled().await },
                                )
                                .await
                            {
                                error!(error = %e, "proxy server error");
                            }
                        })
                    }
                }
            }
            _ => {
                if !tls_cfg.fail_open {
                    anyhow::bail!(
                        "TLS is enabled but tls.cert_path/tls.key_path are not configured, \
                         and tls.fail_open is false"
                    );
                }
                warn!("TLS enabled but cert_path/key_path not configured — using plain TCP");
                let listener = tokio::net::TcpListener::bind(&proxy_listen)
                    .await
                    .with_context(|| format!("binding proxy server to {proxy_listen}"))?;
                info!(addr = %proxy_listen, "proxy server listening");
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener, proxy_app)
                        .with_graceful_shutdown(async move { proxy_token.cancelled().await })
                        .await
                    {
                        error!(error = %e, "proxy server error");
                    }
                })
            }
        }
    } else {
        let listener = tokio::net::TcpListener::bind(&proxy_listen)
            .await
            .with_context(|| format!("binding proxy server to {proxy_listen}"))?;
        info!(addr = %proxy_listen, "proxy server listening");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, proxy_app)
                .with_graceful_shutdown(async move { proxy_token.cancelled().await })
                .await
            {
                error!(error = %e, "proxy server error");
            }
        })
    };

    // 13. Start admin server
    let admin_listen = initial_config.server.admin_listen.clone();
    let admin_state = AdminState {
        cache: Arc::clone(&cache),
        db: db.clone(),
        router: Arc::clone(&shared_router),
        config: Arc::clone(&shared_config),
        authz: Arc::clone(&authz),
        approval_manager: approval_manager.clone(),
        admin_events: Some(admin_event_tx.clone()),
    };

    // Admin-auth posture: enforce when configured; refuse to start if a
    // non-loopback admin bind has no auth (fail-safe against exposing an
    // unauthenticated control plane).
    use flint_gate_core::config::types::AdminAuthPosture;
    let admin_authenticator = match initial_config.server.admin_auth_posture() {
        AdminAuthPosture::Enforce => {
            let provider = &initial_config
                .server
                .admin_auth
                .as_ref()
                .expect("Enforce implies admin_auth is Some")
                .provider;
            let auth = flint_gate_core::auth::build_authenticator(
                "admin",
                provider,
                &http_client,
                db.clone(),
            );
            info!("admin API authentication enabled");
            Some(auth)
        }
        AdminAuthPosture::AllowLoopback => {
            info!(
                addr = %admin_listen,
                "admin API is UNAUTHENTICATED on a loopback bind (dev). Set server.admin_auth before exposing it."
            );
            None
        }
        AdminAuthPosture::RefuseStart => {
            anyhow::bail!(
                "refusing to start: admin_listen ({admin_listen}) is not loopback and server.admin_auth is not set — \
                 this would expose an unauthenticated admin API. Set server.admin_auth or bind admin_listen to loopback."
            );
        }
    };

    // Kubernetes admin-exposure guard: when running inside K8s and admin_auth
    // is not configured, the admin API is reachable cluster-wide via the Service
    // (before NetworkPolicy is applied) or via `kubectl port-forward`. Warn the
    // operator to set server.admin_auth or apply k8s/network-policy.yaml.
    if k8s_admin_unprotected(initial_config.server.admin_auth.is_some()) {
        warn!(
            admin_listen = %admin_listen,
            "running in Kubernetes with no server.admin_auth configured. \
             The admin API (port 4457) is unauthenticated. Apply k8s/network-policy.yaml \
             to deny cluster-wide ingress to port 4457, or set server.admin_auth to \
             require authentication."
        );
    }

    // Derive once: is the admin bind exposed beyond localhost?
    let admin_is_non_loopback = !{
        // Mirror of the private listen_is_loopback() helper in config::types.
        let host = if let Some(rest) = admin_listen.strip_prefix('[') {
            rest.split_once(']').map(|(h, _)| h).unwrap_or("")
        } else {
            admin_listen.rsplit_once(':').map(|(h, _)| h).unwrap_or(admin_listen.as_str())
        };
        host.eq_ignore_ascii_case("localhost")
            || host == "127.0.0.1"
            || host == "::1"
            || host == "0:0:0:0:0:0:0:1"
    };

    // Multi-replica approval-store warning: the ApprovalManager is in-process
    // (per-replica). A decision on the wrong replica returns 404, making
    // approval requests appear permanently stuck. Mitigation: apply
    // k8s/service-admin.yaml which sets sessionAffinity: ClientIP.
    if admin_is_non_loopback {
        let replica_count = multi_replica_count();
        if replica_count.map(|n| n > 1).unwrap_or(false) {
            warn!(
                admin_listen = %admin_listen,
                replica_count = replica_count.unwrap_or(0),
                "MULTI-REPLICA WARNING: approval store is in-process PER REPLICA. \
                 ~50% of approval decisions will land on the wrong replica and return 404. \
                 REQUIRED MITIGATION: apply k8s/service-admin.yaml (sessionAffinity: ClientIP) \
                 to pin each admin client to one replica. \
                 KNOWN LIMITATION: sticky session is lost on pod restart — any pending \
                 approval stream on the restarted pod will be abandoned."
            );
        } else {
            warn!(
                admin_listen = %admin_listen,
                "admin API is bound to a non-loopback address with an in-process approval store. \
                 Approval decisions will fail silently if you scale beyond one replica. \
                 Set REPLICA_COUNT env var and apply k8s/service-admin.yaml \
                 (sessionAffinity: ClientIP) before scaling."
            );
        }
    }

    // CORS warning: the admin router has no CORS config. When the bind is
    // non-loopback, a browser-based operator dashboard hosted on a different
    // origin will hit CORS preflight failures unless the deployment adds a
    // reverse-proxy CORS header or the admin UI and admin API share an origin.
    // The `server.admin_cors` config field is future work; emit an advisory
    // warn now so operators know to handle it at the network layer.
    if admin_is_non_loopback {
        warn!(
            admin_listen = %admin_listen,
            "admin API has no CORS configuration. Browser-based dashboards on a \
             different origin will fail preflight. Add `Access-Control-Allow-Origin` \
             headers at your reverse proxy, or ensure the admin UI is served from \
             the same origin as the admin API."
        );
    }

    // Admin per-replica rate-limit (optional; disabled on loopback-dev by default).
    let admin_rate_layer = initial_config.server.admin_rate_limit.as_ref().and_then(|rl| {
        if !rl.enabled {
            return None;
        }
        match flint_gate_core::ratelimit::build_governor_layer(rl.per_second, rl.burst) {
            Some(layer) => {
                info!(
                    per_second = rl.per_second,
                    burst = rl.burst,
                    "admin in-process rate limiter enabled"
                );
                Some(layer)
            }
            None => {
                warn!("admin_rate_limit enabled but config was degenerate — limiter not applied");
                None
            }
        }
    });

    let admin_app =
        flint_gate_core::admin::admin_router_with_auth(admin_state, admin_authenticator, admin_rate_layer)
            .layer(TraceLayer::new_for_http());
    let admin_listener = tokio::net::TcpListener::bind(&admin_listen)
        .await
        .with_context(|| format!("binding admin server to {admin_listen}"))?;
    info!(addr = %admin_listen, "admin server listening");
    let admin_token = token.clone();
    let mut admin_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(admin_listener, admin_app)
            .with_graceful_shutdown(async move { admin_token.cancelled().await })
            .await
        {
            error!(error = %e, "admin server error");
        }
    });

    // 14. Config hot-reload — re-apply CLI overrides after every file change
    let reload_router = Arc::clone(&shared_router);
    let reload_shared = Arc::clone(&shared_config);
    let reload_db = db.clone();
    let cli_for_reload = cli.clone();
    let mut config_watcher = tokio::spawn(async move {
        while config_rx.changed().await.is_ok() {
            let new_config = apply_overrides(config_rx.borrow().clone(), &cli_for_reload);
            info!("config changed — rebuilding router");
            *reload_shared.write().await = new_config.clone();

            // Merge DB routes if override_yaml is active.
            let r = if new_config.database.override_yaml {
                if let Some(ref d) = reload_db {
                    match d.load_routes().await {
                        Ok(db_routes) => {
                            GateRouter::from_config_and_db_routes(&new_config, &db_routes)
                        }
                        Err(e) => {
                            warn!(error = %e, "failed to reload DB routes on config change — using YAML only");
                            GateRouter::from_config(&new_config)
                        }
                    }
                } else {
                    GateRouter::from_config(&new_config)
                }
            } else {
                GateRouter::from_config(&new_config)
            };

            let n = r.route_count();
            *reload_router.write().await = r;
            info!(route_count = n, "router rebuilt");
        }
        warn!("config watch channel closed");
    });

    // 15. Wait for first unexpected exit or shutdown signal
    tokio::select! {
        _ = &mut proxy_server   => error!("proxy server task exited unexpectedly"),
        _ = &mut admin_server   => error!("admin server task exited unexpectedly"),
        _ = &mut config_watcher => warn!("config watcher task exited"),
        _ = shutdown_signal()   => info!("shutdown signal — draining connections (timeout: {}s)", shutdown_timeout),
    }

    // Fire graceful shutdown on all servers, then wait up to shutdown_timeout.
    token.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(shutdown_timeout), async {
        let _ = tokio::join!(proxy_server, admin_server);
    })
    .await;

    info!("shutdown complete");
    Ok(())
}

/// Assemble the `/oauth/token` + `/oauth/introspect` routes, attaching the
/// shared cross-replica limiter per-endpoint when a Redis limiter is present.
///
/// The introspection oracle (RFC 7662 §2.1) always uses the `Deny` posture on a
/// backend outage; the token endpoint uses the operator-configured posture
/// (`deny` or `degrade`). The in-process governor is layered by the caller over
/// both routes (burst shield + the `degrade` fallback).
#[cfg(feature = "redis-l2")]
fn build_oauth_routes(
    oauth_state: flint_gate_core::auth::oauth::OAuthState,
    orl: &flint_gate_core::config::types::RateLimitConfig,
    shared: Option<flint_gate_core::ratelimit::RedisRateLimiter>,
    token_posture: flint_gate_core::config::types::BackendUnavailablePosture,
) -> Router {
    use flint_gate_core::config::types::BackendUnavailablePosture;
    use flint_gate_core::ratelimit::OAuthLimiter;

    // Attach the shared cross-replica limiter to the OAuthState per endpoint:
    // the token endpoint uses the operator-configured posture; the introspection
    // oracle always denies on a backend outage. The handlers consult the limiter
    // before doing any work (429 over-window; 503/degrade on Redis outage).
    let mut state = oauth_state;
    if orl.enabled {
        if let Some(limiter) = shared {
            state.token_limiter = Some(Arc::new(OAuthLimiter::new(
                limiter.clone(),
                orl.per_second,
                token_posture,
                "token",
            )));
            state.introspect_limiter = Some(Arc::new(OAuthLimiter::new(
                limiter,
                orl.per_second,
                BackendUnavailablePosture::Deny,
                "introspect",
            )));
            info!(
                per_second = orl.per_second,
                token_posture = ?token_posture,
                "OAuth shared cross-replica rate limiter enabled"
            );
        }
    }

    Router::new()
        .route(
            "/oauth/token",
            post(flint_gate_core::auth::oauth::token_endpoint),
        )
        .route(
            "/oauth/introspect",
            post(flint_gate_core::auth::oauth::introspect_endpoint),
        )
        .with_state(state)
}

/// No-`redis-l2` build: routes only, no shared limiter (the in-process governor
/// is still layered by the caller).
#[cfg(not(feature = "redis-l2"))]
fn build_oauth_routes(
    oauth_state: flint_gate_core::auth::oauth::OAuthState,
    _orl: &flint_gate_core::config::types::RateLimitConfig,
) -> Router {
    Router::new()
        .route(
            "/oauth/token",
            post(flint_gate_core::auth::oauth::token_endpoint),
        )
        .route(
            "/oauth/introspect",
            post(flint_gate_core::auth::oauth::introspect_endpoint),
        )
        .with_state(oauth_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flint_gate_core::config::types::{DatabaseConfig, JwtConfig, ServerConfig};
    use std::sync::Mutex;

    // Env-var-touching tests must not run concurrently — process env is global state.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn base_config() -> GateConfig {
        GateConfig {
            server: ServerConfig {
                listen: "0.0.0.0:4456".to_string(),
                admin_listen: "0.0.0.0:4457".to_string(),
                tls: Default::default(),
                shutdown_timeout_secs: 30,
                rate_limit: Default::default(),
                admin_rate_limit: None,
                admin_auth: None,
                allow_insecure_upstream: false,
                strict_agent_governance: false,
                require_policies_at_startup: false,
            },
            database: DatabaseConfig {
                url: "postgres://original".to_string(),
                max_connections: 20,
                override_yaml: false,
            },
            jwt: JwtConfig {
                signing_key_secret: None,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn cli(listen: Option<&str>, db_url: Option<&str>, secret: Option<&str>) -> Cli {
        Cli {
            config: "config.yaml".to_string(),
            listen: listen.map(str::to_string),
            admin_listen: None,
            database_url: db_url.map(str::to_string),
            log: "info".to_string(),
            jwt_secret: secret.map(str::to_string),
            jwt_key_path: None,
        }
    }

    #[test]
    fn cli_listen_wins() {
        let cfg = apply_overrides(base_config(), &cli(Some("0.0.0.0:9000"), None, None));
        assert_eq!(cfg.server.listen, "0.0.0.0:9000");
        assert_eq!(cfg.server.admin_listen, "0.0.0.0:4457"); // untouched
    }
    #[test]
    fn cli_db_url_wins() {
        let cfg = apply_overrides(base_config(), &cli(None, Some("postgres://new"), None));
        assert_eq!(cfg.database.url, "postgres://new");
    }
    #[test]
    fn cli_jwt_secret_wins() {
        let cfg = apply_overrides(base_config(), &cli(None, None, Some("s3cr3t")));
        assert_eq!(cfg.jwt.signing_key_secret.as_deref(), Some("s3cr3t"));
    }
    #[test]
    fn no_flags_preserves_yaml() {
        let cfg = apply_overrides(base_config(), &cli(None, None, None));
        assert_eq!(cfg.server.listen, "0.0.0.0:4456");
        assert_eq!(cfg.database.url, "postgres://original");
    }

    // ── TLS fail_open guard tests ─────────────────────────────────────────────

    #[test]
    fn tls_missing_paths_fail_open_false_errors() {
        use flint_gate_core::config::types::TlsConfig;
        let tls = TlsConfig {
            enabled: true,
            cert_path: None,
            key_path: None,
            fail_open: false,
        };
        let result = check_tls_config(&tls);
        assert!(result.is_err(), "expected error when fail_open=false and paths missing");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("tls.fail_open is false"),
            "error message should mention fail_open: {msg}"
        );
    }

    #[test]
    fn tls_missing_paths_fail_open_true_succeeds() {
        use flint_gate_core::config::types::TlsConfig;
        let tls = TlsConfig {
            enabled: true,
            cert_path: None,
            key_path: None,
            fail_open: true,
        };
        assert!(
            check_tls_config(&tls).is_ok(),
            "fail_open=true should allow missing paths"
        );
    }

    #[test]
    fn tls_disabled_always_succeeds() {
        use flint_gate_core::config::types::TlsConfig;
        let tls = TlsConfig {
            enabled: false,
            cert_path: None,
            key_path: None,
            fail_open: false,
        };
        assert!(check_tls_config(&tls).is_ok());
    }

    #[test]
    fn tls_enabled_with_paths_succeeds() {
        use flint_gate_core::config::types::TlsConfig;
        let tls = TlsConfig {
            enabled: true,
            cert_path: Some("/etc/cert.pem".to_string()),
            key_path: Some("/etc/key.pem".to_string()),
            fail_open: false,
        };
        assert!(check_tls_config(&tls).is_ok());
    }

    // ── K8s admin-exposure guard tests ───────────────────────────────────────

    #[test]
    fn k8s_unprotected_when_in_k8s_and_no_auth() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        let result = k8s_admin_unprotected(false);
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(result, "should warn when in K8s without admin_auth");
    }

    #[test]
    fn k8s_protected_when_auth_configured() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        let result = k8s_admin_unprotected(true);
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(!result, "should not warn when admin_auth is configured");
    }

    #[test]
    fn k8s_not_in_k8s_no_warning() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(
            !k8s_admin_unprotected(false),
            "should not warn when not in K8s"
        );
    }

    // ── Multi-replica sticky-session warning tests ────────────────────────────

    #[test]
    fn multi_replica_count_returns_some_when_set() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("REPLICA_COUNT", "2");
        let count = multi_replica_count();
        std::env::remove_var("REPLICA_COUNT");
        assert_eq!(count, Some(2), "should parse REPLICA_COUNT=2");
    }

    #[test]
    fn multi_replica_count_returns_none_when_unset() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("REPLICA_COUNT");
        assert_eq!(multi_replica_count(), None);
    }

    #[test]
    fn multi_replica_count_returns_none_for_invalid() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("REPLICA_COUNT", "not-a-number");
        let count = multi_replica_count();
        std::env::remove_var("REPLICA_COUNT");
        assert_eq!(count, None, "invalid REPLICA_COUNT should parse as None");
    }

    #[test]
    fn multi_replica_triggers_warning_when_above_one() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("REPLICA_COUNT", "3");
        let count = multi_replica_count();
        std::env::remove_var("REPLICA_COUNT");
        assert!(
            count.map(|n| n > 1).unwrap_or(false),
            "REPLICA_COUNT=3 should trigger multi-replica warning"
        );
    }

    #[test]
    fn single_replica_does_not_trigger_warning() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("REPLICA_COUNT", "1");
        let count = multi_replica_count();
        std::env::remove_var("REPLICA_COUNT");
        assert!(
            !count.map(|n| n > 1).unwrap_or(false),
            "REPLICA_COUNT=1 should not trigger multi-replica warning"
        );
    }

    // ── Rate-limit multi-replica Redis warning tests ──────────────────────────

    #[test]
    fn rate_limit_warning_when_enabled_no_redis_in_k8s() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        let result = rate_limit_needs_redis_warning(true, false);
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(result, "should warn when rate limiting enabled, no Redis, in K8s");
    }

    #[test]
    fn rate_limit_no_warning_when_redis_configured() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        let result = rate_limit_needs_redis_warning(true, true);
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(!result, "should not warn when Redis is configured");
    }

    #[test]
    fn rate_limit_no_warning_when_not_in_k8s() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        let result = rate_limit_needs_redis_warning(true, false);
        assert!(!result, "should not warn when not in Kubernetes");
    }

    #[test]
    fn rate_limit_no_warning_when_rate_limiting_disabled() {
        let _guard = ENV_MUTEX.lock().unwrap();
        std::env::set_var("KUBERNETES_SERVICE_HOST", "10.96.0.1");
        let result = rate_limit_needs_redis_warning(false, false);
        std::env::remove_var("KUBERNETES_SERVICE_HOST");
        assert!(!result, "should not warn when rate limiting is disabled");
    }
}
