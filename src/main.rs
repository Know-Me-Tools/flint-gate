/// Flint Gate — AI-native auth proxy and API gateway.
///
/// Configuration priority (highest → lowest):
///   CLI flags  >  environment variables  >  config.yaml
mod admin;
mod auth;
mod cache;
mod config;
mod db;
mod middleware;
mod proxy;
mod stream;

use crate::admin::{AdminState, admin_router};
use crate::auth::{JwtMinter, SharedJwtMinter, build_authenticators};
use crate::cache::{GateCache, start_cache_invalidation_listener};
use crate::config::{GateConfig, LookupRegistry, load_config};
use crate::db::Database;
use crate::middleware::{AppState, proxy_handler};
use crate::proxy::{Router as GateRouter, SharedRouter};

use anyhow::{Context, Result};
use axum::{Router, routing::any};
use clap::Parser;
use std::sync::Arc;
use tokio::sync::RwLock;
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
#[command(name = "flint-gate", version, about = "AI-native auth proxy and API gateway")]
struct Cli {
    /// Path to the YAML configuration file.
    #[arg(short = 'c', long, env = "FLINT_GATE_CONFIG", default_value = "config.yaml", value_name = "PATH")]
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
    #[arg(long, env = "RUST_LOG", default_value = "info,flint_gate=debug", value_name = "FILTER")]
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
    if let Some(v) = &cli.listen        { cfg.server.listen           = v.clone(); }
    if let Some(v) = &cli.admin_listen  { cfg.server.admin_listen     = v.clone(); }
    if let Some(v) = &cli.database_url  { cfg.database.url            = v.clone(); }
    if let Some(v) = &cli.jwt_secret    { cfg.jwt.signing_key_secret  = Some(v.clone()); }
    if let Some(v) = &cli.jwt_key_path  { cfg.jwt.signing_key_path    = Some(v.clone()); }
    cfg
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

    // 5. Connect to database
    let db = if initial_config.database.url.is_empty() {
        info!("no database URL configured; DB features disabled");
        None
    } else {
        match Database::connect(&initial_config.database.url, initial_config.database.max_connections).await {
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

    let auth_providers = Arc::new(build_authenticators(&initial_config.auth_providers, &http_client, db.clone()));
    info!(count = auth_providers.len(), "auth providers initialized");

    // 7. Build JWT minter
    let jwt_minter: SharedJwtMinter = Arc::new(RwLock::new(None));
    let jwt_cfg = &initial_config.jwt;
    if jwt_cfg.signing_key_secret.is_some() || jwt_cfg.signing_key_path.is_some() {
        match JwtMinter::from_config(jwt_cfg).await {
            Ok(m) => { *jwt_minter.write().await = Some(m); info!(algorithm = %jwt_cfg.signing_algorithm, "JWT minter initialized"); }
            Err(e) => warn!(error = %e, "JWT minter init failed; minting disabled"),
        }
    } else {
        info!("no JWT signing key configured; JWT minting disabled");
    }

    // 8. Build router
    let gate_router = GateRouter::from_config(&initial_config);
    if initial_config.database.override_yaml {
        if let Some(ref d) = db {
            match d.load_routes().await {
                Ok(routes) => info!(count = routes.len(), "DB routes available (override mode)"),
                Err(e)     => warn!(error = %e, "failed to load DB routes"),
            }
        }
    }
    info!(route_count = gate_router.route_count(), "route table built");
    let shared_router: SharedRouter = Arc::new(RwLock::new(gate_router));

    // 9. Build cache + LISTEN/NOTIFY invalidation
    let cache = Arc::new(GateCache::from_config(&initial_config.cache));
    if let Some(ref d) = db {
        let ch = initial_config.cache.invalidation_channel.clone();
        start_cache_invalidation_listener(d.pool(), Arc::clone(&cache), ch).await;
    }

    // 10. Build lookup registry
    let lookup_registry = Arc::new(LookupRegistry::new(db.clone()));

    // 11. Assemble AppState
    let app_state = Arc::new(AppState {
        config:          Arc::clone(&shared_config),
        router:          Arc::clone(&shared_router),
        auth_providers:  Arc::clone(&auth_providers),
        jwt_minter:      Arc::clone(&jwt_minter),
        cache:           Arc::clone(&cache),
        db:              db.clone(),
        http_client:     http_client.clone(),
        lookup_registry: Arc::clone(&lookup_registry),
    });

    // 12. Start proxy server
    let proxy_listen = initial_config.server.listen.clone();
    let proxy_app = Router::new()
        .fallback(any(proxy_handler))
        .with_state(Arc::clone(&app_state))
        .layer(TraceLayer::new_for_http());
    let proxy_listener = tokio::net::TcpListener::bind(&proxy_listen)
        .await
        .with_context(|| format!("binding proxy server to {proxy_listen}"))?;
    info!(addr = %proxy_listen, "proxy server listening");
    let proxy_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(proxy_listener, proxy_app).await { error!(error = %e, "proxy server error"); }
    });

    // 13. Start admin server
    let admin_listen = initial_config.server.admin_listen.clone();
    let admin_state = AdminState { cache: Arc::clone(&cache), db: db.clone(), router: Arc::clone(&shared_router), config: Arc::clone(&shared_config) };
    let admin_app = admin_router(admin_state).layer(TraceLayer::new_for_http());
    let admin_listener = tokio::net::TcpListener::bind(&admin_listen)
        .await
        .with_context(|| format!("binding admin server to {admin_listen}"))?;
    info!(addr = %admin_listen, "admin server listening");
    let admin_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(admin_listener, admin_app).await { error!(error = %e, "admin server error"); }
    });

    // 14. Config hot-reload — re-apply CLI overrides after every file change
    let reload_router  = Arc::clone(&shared_router);
    let reload_shared  = Arc::clone(&shared_config);
    let cli_for_reload = cli.clone();
    let config_watcher = tokio::spawn(async move {
        while config_rx.changed().await.is_ok() {
            let new_config = apply_overrides(config_rx.borrow().clone(), &cli_for_reload);
            info!("config changed — rebuilding router");
            *reload_shared.write().await = new_config.clone();
            let r = GateRouter::from_config(&new_config);
            let n = r.route_count();
            *reload_router.write().await = r;
            info!(route_count = n, "router rebuilt");
        }
        warn!("config watch channel closed");
    });

    tokio::select! {
        _ = proxy_server   => error!("proxy server task exited"),
        _ = admin_server   => error!("admin server task exited"),
        _ = config_watcher => warn!("config watcher task exited"),
        _ = tokio::signal::ctrl_c() => info!("received SIGINT — shutting down"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{DatabaseConfig, JwtConfig, ServerConfig};

    fn base_config() -> GateConfig {
        GateConfig {
            server: ServerConfig { listen: "0.0.0.0:4456".to_string(), admin_listen: "0.0.0.0:4457".to_string(), tls: Default::default() },
            database: DatabaseConfig { url: "postgres://original".to_string(), max_connections: 20, override_yaml: false },
            jwt: JwtConfig { signing_key_secret: None, ..Default::default() },
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

    #[test] fn cli_listen_wins() {
        let cfg = apply_overrides(base_config(), &cli(Some("0.0.0.0:9000"), None, None));
        assert_eq!(cfg.server.listen, "0.0.0.0:9000");
        assert_eq!(cfg.server.admin_listen, "0.0.0.0:4457"); // untouched
    }
    #[test] fn cli_db_url_wins() {
        let cfg = apply_overrides(base_config(), &cli(None, Some("postgres://new"), None));
        assert_eq!(cfg.database.url, "postgres://new");
    }
    #[test] fn cli_jwt_secret_wins() {
        let cfg = apply_overrides(base_config(), &cli(None, None, Some("s3cr3t")));
        assert_eq!(cfg.jwt.signing_key_secret.as_deref(), Some("s3cr3t"));
    }
    #[test] fn no_flags_preserves_yaml() {
        let cfg = apply_overrides(base_config(), &cli(None, None, None));
        assert_eq!(cfg.server.listen, "0.0.0.0:4456");
        assert_eq!(cfg.database.url, "postgres://original");
    }
}
