/// Flint Gate — AI-native auth proxy and API gateway.
///
/// Strike an idea. Watch it build.
///
/// Startup sequence:
/// 1. Init structured tracing
/// 2. Load YAML config (with hot-reload watcher)
/// 3. Connect to Postgres (if configured)
/// 4. Build authenticator map from config
/// 5. Build JWT minter (if configured)
/// 6. Build route router (YAML + DB if override enabled)
/// 7. Create shared AppState
/// 8. Start proxy server (`:4456`)
/// 9. Start admin server (`:4457`)
/// 10. Watch for config changes and reload router
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
use crate::config::{load_config, SharedConfig};
use crate::db::Database;
use crate::middleware::{AppState, proxy_handler};
use crate::proxy::{Router as GateRouter, SharedRouter};

use anyhow::{Context, Result};
use axum::{Router, routing::any};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Init tracing ────────────────────────────────────────────────────
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,flint_gate=debug"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .init();

    info!("⚡ Flint Gate starting — Strike an idea. Watch it build.");

    // ── 2. Load config ─────────────────────────────────────────────────────
    let config_path = std::env::var("FLINT_GATE_CONFIG")
        .unwrap_or_else(|_| "config.yaml".to_string());

    let (shared_config, mut config_rx) = load_config(&config_path)
        .await
        .with_context(|| format!("loading config from {config_path}"))?;

    let initial_config = shared_config.read().await.clone();
    info!(
        proxy_listen = %initial_config.server.listen,
        admin_listen = %initial_config.server.admin_listen,
        "config loaded"
    );

    // ── 3. Connect to database ─────────────────────────────────────────────
    let db_url = std::env::var("DATABASE_URL")
        .ok()
        .or_else(|| {
            if initial_config.database.url.is_empty() {
                None
            } else {
                Some(initial_config.database.url.clone())
            }
        });

    let db = if let Some(url) = db_url {
        match Database::connect(&url, initial_config.database.max_connections).await {
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
    } else {
        info!("no database URL configured; DB features disabled");
        None
    };

    // ── 4. Build authenticators ────────────────────────────────────────────
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("building HTTP client")?;

    let auth_providers = Arc::new(build_authenticators(
        &initial_config.auth_providers,
        &http_client,
    ));
    info!(count = auth_providers.len(), "auth providers initialized");

    // ── 5. Build JWT minter ────────────────────────────────────────────────
    let jwt_minter: SharedJwtMinter = Arc::new(RwLock::new(None));

    // Only build if signing key is configured
    let jwt_cfg = &initial_config.jwt;
    if jwt_cfg.signing_key_secret.is_some() || jwt_cfg.signing_key_path.is_some() {
        match JwtMinter::from_config(jwt_cfg).await {
            Ok(minter) => {
                *jwt_minter.write().await = Some(minter);
                info!(algorithm = %jwt_cfg.signing_algorithm, "JWT minter initialized");
            }
            Err(e) => {
                warn!(error = %e, "JWT minter initialization failed; minting disabled");
            }
        }
    } else {
        info!("no JWT signing key configured; JWT minting disabled");
    }

    // ── 6. Build router ────────────────────────────────────────────────────
    let mut gate_router = GateRouter::from_config(&initial_config);

    // Merge DB routes if override is enabled
    if initial_config.database.override_yaml {
        if let Some(ref database) = db {
            match database.load_routes().await {
                Ok(db_routes) => {
                    info!(count = db_routes.len(), "DB routes loaded (override mode)");
                    // TODO: merge DB routes into the router in Phase 2 completion
                }
                Err(e) => {
                    warn!(error = %e, "failed to load DB routes");
                }
            }
        }
    }

    info!(
        route_count = gate_router.route_count(),
        "route table built"
    );

    let shared_router: SharedRouter = Arc::new(RwLock::new(gate_router));

    // ── 7. Build cache ─────────────────────────────────────────────────────
    let cache = Arc::new(GateCache::from_config(&initial_config.cache));

    // Start LISTEN/NOTIFY cache invalidation if DB is connected
    if let Some(ref database) = db {
        let channel = initial_config.cache.invalidation_channel.clone();
        start_cache_invalidation_listener(database.pool(), Arc::clone(&cache), channel).await;
    }

    // ── 8. Assemble AppState ───────────────────────────────────────────────
    let app_state = Arc::new(AppState {
        config: Arc::clone(&shared_config),
        router: Arc::clone(&shared_router),
        auth_providers: Arc::clone(&auth_providers),
        jwt_minter: Arc::clone(&jwt_minter),
        cache: Arc::clone(&cache),
        db: db.clone(),
        http_client: http_client.clone(),
    });

    // ── 9. Start proxy server ──────────────────────────────────────────────
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
        if let Err(e) = axum::serve(proxy_listener, proxy_app).await {
            error!(error = %e, "proxy server error");
        }
    });

    // ── 10. Start admin server ─────────────────────────────────────────────
    let admin_listen = initial_config.server.admin_listen.clone();
    let admin_state = AdminState {
        cache: Arc::clone(&cache),
        db: db.clone(),
        router: Arc::clone(&shared_router),
        config: Arc::clone(&shared_config),
    };
    let admin_app = admin_router(admin_state).layer(TraceLayer::new_for_http());

    let admin_listener = tokio::net::TcpListener::bind(&admin_listen)
        .await
        .with_context(|| format!("binding admin server to {admin_listen}"))?;

    info!(addr = %admin_listen, "admin server listening");

    let admin_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(admin_listener, admin_app).await {
            error!(error = %e, "admin server error");
        }
    });

    // ── 11. Config hot-reload watcher ──────────────────────────────────────
    let reload_router = Arc::clone(&shared_router);
    let reload_config = Arc::clone(&shared_config);
    let reload_http_client = http_client.clone();

    let config_watcher = tokio::spawn(async move {
        while config_rx.changed().await.is_ok() {
            let new_config = config_rx.borrow().clone();
            info!("config changed — rebuilding router");

            let new_router = GateRouter::from_config(&new_config);
            let new_route_count = new_router.route_count();
            *reload_router.write().await = new_router;

            info!(route_count = new_route_count, "router rebuilt from updated config");
        }
        warn!("config watch channel closed");
    });

    // ── Wait for all tasks ─────────────────────────────────────────────────
    tokio::select! {
        _ = proxy_server => error!("proxy server task exited"),
        _ = admin_server => error!("admin server task exited"),
        _ = config_watcher => warn!("config watcher task exited"),
        _ = tokio::signal::ctrl_c() => {
            info!("received SIGINT — shutting down");
        }
    }

    Ok(())
}
