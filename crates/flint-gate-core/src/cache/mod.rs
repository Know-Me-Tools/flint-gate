/// In-process cache using `moka` with Postgres LISTEN/NOTIFY invalidation.
///
/// Three cache tiers:
/// - `routes` — compiled route configs (invalidated on config change)
/// - `sessions` — Kratos session validation results (keyed by SHA-256 of credential)
/// - `kv` — generic key-value for API keys, JWKs, etc.
///
/// Optional Redis L2 cache when `redis-l2` feature is enabled.
use crate::auth::identity::Identity;
use crate::config::types::CacheConfig;
use moka::future::Cache;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

#[cfg(feature = "redis-l2")]
use redis::AsyncCommands;

/// The central cache store.
#[derive(Clone)]
pub struct GateCache {
    /// Route configuration cache keyed by route ID.
    pub routes: Cache<String, Value>,
    /// Session validation cache keyed by session token/cookie.
    pub sessions: Cache<String, Value>,
    /// Generic KV cache.
    pub kv: Cache<String, String>,
    /// Optional Redis L2 cache manager.
    #[cfg(feature = "redis-l2")]
    l2: Option<redis::aio::ConnectionManager>,
}

/// Redis key prefix for L2 cache entries.
#[cfg(feature = "redis-l2")]
const L2_PREFIX: &str = "flint";

impl GateCache {
    /// Build a cache from config.
    pub fn from_config(cfg: &CacheConfig) -> Self {
        let ttl = Duration::from_secs(cfg.l1.ttl_seconds);
        let max = cfg.l1.max_capacity;

        let routes = Cache::builder()
            .max_capacity(max / 10) // routes are fewer but heavier
            .time_to_live(ttl)
            .build();

        let sessions = Cache::builder().max_capacity(max).time_to_live(ttl).build();

        let kv = Cache::builder()
            .max_capacity(max / 2)
            .time_to_live(ttl)
            .build();

        Self {
            routes,
            sessions,
            kv,
            #[cfg(feature = "redis-l2")]
            l2: None,
        }
    }

    /// Connect to Redis L2 cache when configured.
    #[cfg(feature = "redis-l2")]
    pub async fn connect_l2(&mut self, cfg: &CacheConfig) -> anyhow::Result<()> {
        if cfg.l2.enabled {
            if let Some(ref url) = cfg.l2.redis_url {
                if !url.is_empty() {
                    let client = redis::Client::open(url.as_str())?;
                    let manager = client.get_connection_manager().await?;
                    self.l2 = Some(manager);
                    info!(redis_url = %url, "Redis L2 cache connected");
                }
            }
        }
        Ok(())
    }

    /// Return a clone of the Redis L2 connection manager, if connected.
    ///
    /// Reused by the rate-limit module so it shares the single connection
    /// manager/pool established by [`GateCache::connect_l2`] rather than
    /// opening a second connection.
    #[cfg(feature = "redis-l2")]
    pub fn l2_connection(&self) -> Option<redis::aio::ConnectionManager> {
        self.l2.clone()
    }

    /// Invalidate all cache entries. Called on config change.
    pub async fn invalidate_all(&self) {
        self.routes.invalidate_all();
        self.sessions.invalidate_all();
        self.kv.invalidate_all();

        #[cfg(feature = "redis-l2")]
        {
            if let Some(ref con) = self.l2 {
                // SCAN + DEL by prefix (non-blocking, best-effort)
                let pattern = format!("{L2_PREFIX}:*");
                let mut con = con.clone();
                match Self::scan_and_del(&mut con, &pattern).await {
                    Ok(n) => info!(deleted = n, "L2 Redis entries invalidated"),
                    Err(e) => warn!(error = %e, "L2 Redis invalidation failed"),
                }
            }
        }

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
    /// L1 (moka) miss → L2 (Redis) GET → back-fill L1.
    pub async fn get_session(&self, credential: &str) -> Option<Identity> {
        let key = Self::session_key(credential);

        // L1 check
        if let Some(value) = self.sessions.get(&key).await {
            match serde_json::from_value(value) {
                Ok(identity) => {
                    debug!(key = %key, "session L1 cache hit");
                    return Some(identity);
                }
                Err(e) => debug!(error = %e, "failed to deserialize L1 cached session"),
            }
        }

        // L2 check (Redis)
        #[cfg(feature = "redis-l2")]
        {
            if let Some(ref con) = self.l2 {
                let redis_key = Self::l2_session_key(&key);
                let mut con = con.clone();
                match con.get::<_, Option<String>>(&redis_key).await {
                    Ok(Some(json_str)) => {
                        match serde_json::from_str::<Identity>(&json_str) {
                            Ok(identity) => {
                                debug!(key = %key, "session L2 cache hit");
                                // Back-fill L1
                                if let Ok(value) = serde_json::to_value(&identity) {
                                    self.sessions.insert(key, value).await;
                                }
                                return Some(identity);
                            }
                            Err(e) => debug!(error = %e, "failed to deserialize L2 cached session"),
                        }
                    }
                    Ok(None) => debug!(key = %key, "session L2 cache miss"),
                    Err(e) => warn!(error = %e, "L2 Redis GET failed"),
                }
            }
        }

        None
    }

    /// Store an authenticated identity in the session cache.
    /// Writes to both L1 (moka) and L2 (Redis) when available.
    pub async fn put_session(&self, credential: &str, identity: &Identity) {
        let key = Self::session_key(credential);

        // L1 write
        match serde_json::to_value(identity) {
            Ok(value) => {
                // L2 write (best-effort)
                #[cfg(feature = "redis-l2")]
                {
                    if let Some(ref con) = self.l2 {
                        let redis_key = Self::l2_session_key(&key);
                        if let Ok(json_str) = serde_json::to_string(identity) {
                            let mut con = con.clone();
                            let ttl = 60u64; // L2 TTL in seconds
                            if let Err(e) = con.set_ex::<_, _, ()>(&redis_key, &json_str, ttl).await
                            {
                                warn!(error = %e, "L2 Redis SET failed");
                            }
                        }
                    }
                }
                self.sessions.insert(key, value).await;
            }
            Err(e) => debug!(error = %e, "failed to serialize identity for session cache"),
        }
    }

    /// Invalidate a specific session cache entry by raw credential.
    #[allow(dead_code)]
    pub async fn invalidate_session(&self, credential: &str) {
        let key = Self::session_key(credential);
        self.sessions.invalidate(&key).await;

        #[cfg(feature = "redis-l2")]
        {
            if let Some(ref con) = self.l2 {
                let redis_key = Self::l2_session_key(&key);
                let mut con = con.clone();
                let _: Result<(), _> = con.del(&redis_key).await;
            }
        }
    }

    /// SHA-256 of the credential — the cache key for a session.
    fn session_key(credential: &str) -> String {
        let mut h = Sha256::new();
        h.update(credential.as_bytes());
        hex::encode(h.finalize())
    }

    /// Redis key for a session: `flint:session:<sha256>`.
    #[cfg(feature = "redis-l2")]
    fn l2_session_key(hash: &str) -> String {
        format!("{L2_PREFIX}:session:{hash}")
    }

    /// SCAN + DEL all keys matching a pattern (used by invalidate_all).
    #[cfg(feature = "redis-l2")]
    async fn scan_and_del(
        con: &mut redis::aio::ConnectionManager,
        pattern: &str,
    ) -> anyhow::Result<usize> {
        let mut deleted = 0;
        let mut batch: Vec<String> = Vec::with_capacity(100);
        let mut cursor: u64 = 0;

        loop {
            let result: (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(con)
                .await?;

            cursor = result.0;
            batch.extend(result.1);

            if batch.len() >= 50 {
                deleted += Self::del_batch(con, &batch).await?;
                batch.clear();
            }

            if cursor == 0 {
                break;
            }
        }

        if !batch.is_empty() {
            deleted += Self::del_batch(con, &batch).await?;
        }

        Ok(deleted)
    }

    /// Delete a batch of keys.
    #[cfg(feature = "redis-l2")]
    async fn del_batch(
        con: &mut redis::aio::ConnectionManager,
        keys: &[String],
    ) -> anyhow::Result<usize> {
        if keys.is_empty() {
            return Ok(0);
        }
        let count: usize = redis::cmd("DEL").arg(keys).query_async(con).await?;
        Ok(count)
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

/// The action a NOTIFY payload maps to. Extracted so the dispatch logic is
/// unit-testable without a live Postgres LISTEN/NOTIFY connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotifyAction {
    /// Route/site change — invalidate route cache and rebuild the router.
    Routes,
    /// Authorization policy change — reload the Cedar engine from the DB.
    Policies,
    /// Anything else — conservatively flush all caches.
    All,
}

/// Classify a raw NOTIFY payload into a [`NotifyAction`].
pub(crate) fn classify_notification(payload: &str) -> NotifyAction {
    match payload {
        "routes" | "sites" => NotifyAction::Routes,
        "policies" => NotifyAction::Policies,
        _ => NotifyAction::All,
    }
}

/// Start the Postgres LISTEN/NOTIFY cache invalidation listener.
///
/// Subscribes to the configured channel and invalidates caches when a
/// notification arrives. When `db` and `router` are provided and a "routes"
/// notification is received, the router is rebuilt from the DB + YAML config.
/// When `authz` is provided and a "policies" notification is received, the
/// shared authorization engine is reloaded from the database (parse-before-swap,
/// retain last-good) so peer replicas pick up policy edits WITHOUT a restart.
/// This is best-effort — errors are logged, not fatal.
pub async fn start_cache_invalidation_listener(
    pool: sqlx::PgPool,
    cache: Arc<GateCache>,
    channel: String,
    router: Option<(
        crate::proxy::SharedRouter,
        crate::config::SharedConfig,
        Arc<crate::db::Database>,
    )>,
    authz: Option<(Arc<crate::authz::AuthzEngine>, Arc<crate::db::Database>)>,
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
                    match classify_notification(payload) {
                        NotifyAction::Routes => {
                            cache.invalidate_routes().await;
                            // Rebuild router from DB + YAML when override mode is active.
                            if let Some((ref shared_router, ref shared_config, ref db)) = router {
                                rebuild_router_from_db(shared_router, shared_config, db).await;
                            }
                        }
                        NotifyAction::Policies => {
                            // Reload the shared Cedar engine from the DB. Reload
                            // is parse-before-swap + lenient (skip bad rows), so a
                            // poisoned remote row can neither blank nor over-open
                            // this replica's policy bundle.
                            if let Some((ref authz, ref db)) = authz {
                                if let Err(e) = authz.reload_from_database(db).await {
                                    error!(error = %e, "policy reload from NOTIFY failed — retaining last-good");
                                }
                            } else {
                                // No engine wired (no DB / authz on this replica);
                                // fall back to a cache flush for safety.
                                cache.invalidate_all().await;
                            }
                        }
                        NotifyAction::All => cache.invalidate_all().await,
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

        cache
            .kv
            .insert("key".to_string(), "value".to_string())
            .await;
        cache.invalidate_all().await;
        assert!(cache.kv.get("key").await.is_none());
    }

    // ── C1: NOTIFY payload dispatch (the arm the listener selects) ───────────

    #[test]
    fn classify_routes_and_sites_map_to_routes() {
        assert_eq!(classify_notification("routes"), NotifyAction::Routes);
        assert_eq!(classify_notification("sites"), NotifyAction::Routes);
    }

    #[test]
    fn classify_policies_maps_to_policies_reload() {
        // This is the fix for C1: a "policies" NOTIFY must trigger an authz
        // reload, NOT fall through to the generic invalidate_all arm.
        assert_eq!(classify_notification("policies"), NotifyAction::Policies);
    }

    #[test]
    fn classify_unknown_payload_falls_back_to_all() {
        assert_eq!(classify_notification("signing_keys"), NotifyAction::All);
        assert_eq!(classify_notification("something_else"), NotifyAction::All);
        assert_eq!(classify_notification(""), NotifyAction::All);
    }
}
