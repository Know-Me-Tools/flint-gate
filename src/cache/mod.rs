/// In-process cache using `moka` with Postgres LISTEN/NOTIFY invalidation.
///
/// Three cache tiers:
/// - `routes` — compiled route configs (invalidated on config change)
/// - `sessions` — Kratos session validation results (keyed by SHA-256 of credential)
/// - `kv` — generic key-value for API keys, JWKs, etc.
use crate::auth::identity::Identity;
use crate::config::types::CacheConfig;
use moka::future::Cache;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

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

    /// Look up a cached session by raw credential (cookie or bearer token).
    ///
    /// Hashes the credential so no raw token is ever stored in memory.
    pub async fn get_session(&self, credential: &str) -> Option<Identity> {
        let key = Self::session_key(credential);
        let value = self.sessions.get(&key).await?;
        match serde_json::from_value(value) {
            Ok(identity) => {
                debug!(key = %key, "session cache hit");
                Some(identity)
            }
            Err(e) => {
                debug!(error = %e, "failed to deserialize cached session");
                None
            }
        }
    }

    /// Store an authenticated identity in the session cache.
    pub async fn put_session(&self, credential: &str, identity: &Identity) {
        let key = Self::session_key(credential);
        match serde_json::to_value(identity) {
            Ok(value) => { self.sessions.insert(key, value).await; }
            Err(e) => debug!(error = %e, "failed to serialize identity for session cache"),
        }
    }

    /// Invalidate a specific session cache entry by raw credential.
    #[allow(dead_code)]
    pub async fn invalidate_session(&self, credential: &str) {
        let key = Self::session_key(credential);
        self.sessions.invalidate(&key).await;
    }

    /// SHA-256 of the credential — the cache key for a session.
    fn session_key(credential: &str) -> String {
        let mut h = Sha256::new();
        h.update(credential.as_bytes());
        hex::encode(h.finalize())
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
/// notification arrives. When `db` and `router` are provided and a "routes"
/// notification is received, the router is rebuilt from the DB + YAML config.
/// This is best-effort — errors are logged, not fatal.
pub async fn start_cache_invalidation_listener(
    pool: sqlx::PgPool,
    cache: Arc<GateCache>,
    channel: String,
    router: Option<(crate::proxy::SharedRouter, crate::config::SharedConfig, Arc<crate::db::Database>)>,
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
                        "routes" | "sites" => {
                            cache.invalidate_routes().await;
                            // Rebuild router from DB + YAML when override mode is active.
                            if let Some((ref shared_router, ref shared_config, ref db)) = router {
                                rebuild_router_from_db(shared_router, shared_config, db).await;
                            }
                        }
                        _ => cache.invalidate_all().await,
                    }
                }
                Err(e) => {
                    warn!(error = %e, "LISTEN/NOTIFY error; will reconnect");
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

/// Reload DB routes and rebuild the shared router.
async fn rebuild_router_from_db(
    shared_router: &crate::proxy::SharedRouter,
    shared_config: &crate::config::SharedConfig,
    db: &crate::db::Database,
) {
    let config = shared_config.read().await.clone();
    match db.load_routes().await {
        Ok(db_routes) => {
            let new_router = crate::proxy::Router::from_config_and_db_routes(&config, &db_routes);
            let count = new_router.route_count();
            *shared_router.write().await = new_router;
            info!(route_count = count, "router rebuilt from DB + YAML routes");
        }
        Err(e) => {
            warn!(error = %e, "failed to reload DB routes — router unchanged");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::CacheConfig;

    #[tokio::test]
    async fn cache_basic_operations() {
        use crate::auth::identity::Identity;

        let cfg = CacheConfig::default();
        let cache = GateCache::from_config(&cfg);

        let identity = Identity::anonymous("u1");
        cache.put_session("token-1", &identity).await;
        assert!(cache.get_session("token-1").await.is_some());

        cache.invalidate_session("token-1").await;
        assert!(cache.get_session("token-1").await.is_none());
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
