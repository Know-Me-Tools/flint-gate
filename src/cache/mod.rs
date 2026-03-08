/// In-process cache using `moka` with Postgres LISTEN/NOTIFY invalidation.
///
/// Three cache tiers:
/// - `routes` — compiled route configs (invalidated on config change)
/// - `sessions` — Kratos session validation results
/// - `kv` — generic key-value for API keys, JWKs, etc.
use crate::config::types::CacheConfig;
use moka::future::Cache;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// The central cache store.
#[derive(Clone)]
pub struct GateCache {
    /// Route configuration cache keyed by route ID.
    pub routes: Cache<String, Value>,
    /// Session validation cache keyed by session token/cookie.
    pub sessions: Cache<String, Value>,
    /// Generic KV cache.
    pub kv: Cache<String, String>,
}

impl GateCache {
    /// Build a cache from config.
    pub fn from_config(cfg: &CacheConfig) -> Self {
        let ttl = Duration::from_secs(cfg.l1.ttl_seconds);
        let max = cfg.l1.max_capacity;

        let routes = Cache::builder()
            .max_capacity(max / 10) // routes are fewer but heavier
            .time_to_live(ttl)
            .build();

        let sessions = Cache::builder()
            .max_capacity(max)
            .time_to_live(ttl)
            .build();

        let kv = Cache::builder()
            .max_capacity(max / 2)
            .time_to_live(ttl)
            .build();

        Self { routes, sessions, kv }
    }

    /// Invalidate all cache entries. Called on config change.
    pub async fn invalidate_all(&self) {
        self.routes.invalidate_all();
        self.sessions.invalidate_all();
        self.kv.invalidate_all();
        info!("all caches invalidated");
    }

    /// Invalidate the routes cache only.
    pub async fn invalidate_routes(&self) {
        self.routes.invalidate_all();
        info!("routes cache invalidated");
    }

    /// Invalidate a specific session cache entry.
    pub async fn invalidate_session(&self, key: &str) {
        self.sessions.invalidate(key).await;
    }

    /// Cache statistics for the admin API.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            routes_entry_count: self.routes.entry_count(),
            sessions_entry_count: self.sessions.entry_count(),
            kv_entry_count: self.kv.entry_count(),
        }
    }
}

/// Snapshot of cache entry counts.
#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    pub routes_entry_count: u64,
    pub sessions_entry_count: u64,
    pub kv_entry_count: u64,
}

/// Start the Postgres LISTEN/NOTIFY cache invalidation listener.
///
/// Subscribes to the configured channel and invalidates caches when a
/// notification arrives. This is best-effort — errors are logged, not fatal.
pub async fn start_cache_invalidation_listener(
    pool: sqlx::PgPool,
    cache: Arc<GateCache>,
    channel: String,
) {
    tokio::spawn(async move {
        let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, "failed to create PG listener for cache invalidation");
                return;
            }
        };

        if let Err(e) = listener.listen(&channel).await {
            error!(error = %e, channel = %channel, "failed to LISTEN on PG channel");
            return;
        }

        info!(channel = %channel, "listening for cache invalidation notifications");

        loop {
            match listener.recv().await {
                Ok(notification) => {
                    let payload = notification.payload();
                    info!(channel = %channel, payload = %payload, "cache invalidation notification received");
                    match payload {
                        "routes" | "sites" => cache.invalidate_routes().await,
                        _ => cache.invalidate_all().await,
                    }
                }
                Err(e) => {
                    warn!(error = %e, "LISTEN/NOTIFY error; will reconnect");
                    // Attempt reconnect
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if let Err(e) = listener.listen(&channel).await {
                        error!(error = %e, "failed to re-LISTEN after error");
                        return;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::CacheConfig;

    #[tokio::test]
    async fn cache_basic_operations() {
        let cfg = CacheConfig::default();
        let cache = GateCache::from_config(&cfg);

        cache.sessions.insert("token-1".to_string(), serde_json::json!({"id": "u1"})).await;
        assert!(cache.sessions.get("token-1").await.is_some());

        cache.invalidate_session("token-1").await;
        assert!(cache.sessions.get("token-1").await.is_none());
    }

    #[tokio::test]
    async fn invalidate_all_clears_caches() {
        let cfg = CacheConfig::default();
        let cache = GateCache::from_config(&cfg);

        cache.kv.insert("key".to_string(), "value".to_string()).await;
        cache.invalidate_all().await;
        assert!(cache.kv.get("key").await.is_none());
    }
}
