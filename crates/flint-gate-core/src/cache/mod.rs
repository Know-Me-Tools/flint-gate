/// In-process cache using `moka` with Postgres LISTEN/NOTIFY invalidation.
///
/// Three cache tiers:
/// - `routes` — compiled route configs (invalidated on config change)
/// - `sessions` — Kratos session validation results (keyed by SHA-256 of credential)
/// - `kv` — generic key-value for API keys, JWKs, etc.
///
/// Optional Redis L2 cache when `redis-l2` feature is enabled.
use crate::{
    authority::{AuthorityCacheMode, AuthorityReadiness},
    config::types::CacheConfig,
};
use moka::future::Cache;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tracing::{error, info, warn};

#[cfg(feature = "redis-l2")]
mod redis_fence;
mod session;

#[cfg(feature = "redis-l2")]
pub use redis_fence::{
    AuthorityCacheError, AuthoritySessionCacheKey, CredentialKind, RedisAuthorityFence,
};

/// The central cache store.
#[derive(Clone)]
pub struct GateCache {
    /// Route configuration cache keyed by route ID.
    pub routes: Cache<String, Value>,
    /// Session validation cache keyed by session token/cookie.
    pub sessions: Cache<String, Value>,
    /// Generic KV cache.
    pub kv: Cache<String, String>,
    session_generation: Arc<AtomicU64>,
    configuration_revision: Arc<AtomicU64>,
    configuration_snapshot_lock: Arc<tokio::sync::RwLock<()>>,
    session_ttl: Duration,
    authority_readiness: Option<AuthorityReadiness>,
    /// Optional Redis L2 cache manager.
    #[cfg(feature = "redis-l2")]
    l2: Option<redis::aio::ConnectionManager>,
    #[cfg(feature = "redis-l2")]
    l2_client: Option<redis::Client>,
    #[cfg(feature = "redis-l2")]
    authority_fence: Option<Arc<RedisAuthorityFence>>,
    #[cfg(feature = "redis-l2")]
    authority_config_epoch: Arc<AtomicU64>,
}

/// Redis key prefix for L2 cache entries.
#[cfg(feature = "redis-l2")]
const L2_PREFIX: &str = "flint";

#[cfg(feature = "redis-l2")]
const CONFIG_EPOCH_UNINITIALIZED: u64 = u64::MAX;

#[cfg(feature = "redis-l2")]
const CONFIG_EPOCH_INVALID: u64 = u64::MAX - 1;

const CONFIG_REVISION_UNINITIALIZED: u64 = u64::MAX;

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
            session_generation: Arc::new(AtomicU64::new(0)),
            configuration_revision: Arc::new(AtomicU64::new(CONFIG_REVISION_UNINITIALIZED)),
            configuration_snapshot_lock: Arc::new(tokio::sync::RwLock::new(())),
            session_ttl: ttl,
            authority_readiness: None,
            #[cfg(feature = "redis-l2")]
            l2: None,
            #[cfg(feature = "redis-l2")]
            l2_client: None,
            #[cfg(feature = "redis-l2")]
            authority_fence: None,
            #[cfg(feature = "redis-l2")]
            authority_config_epoch: Arc::new(AtomicU64::new(CONFIG_EPOCH_UNINITIALIZED)),
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
                    self.l2_client = Some(client);
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

    #[cfg(feature = "redis-l2")]
    pub fn l2_client(&self) -> Option<redis::Client> {
        self.l2_client.clone()
    }

    #[cfg(feature = "redis-l2")]
    pub fn set_redis_authority_fence(&mut self, fence: Arc<RedisAuthorityFence>) {
        self.authority_fence = Some(fence);
    }

    /// Attach the process-local ASO authority readiness gate before sharing the cache.
    pub fn set_authority_readiness(&mut self, readiness: AuthorityReadiness) {
        self.authority_readiness = Some(readiness);
    }

    /// Cache use is allowed only while this process has a fresh ASO high-water probe.
    pub fn authority_cache_ready(&self) -> bool {
        self.authority_readiness
            .as_ref()
            .is_some_and(AuthorityReadiness::cache_ready)
    }

    pub fn authority_readiness(&self) -> Option<AuthorityReadiness> {
        self.authority_readiness.clone()
    }

    /// Hold this guard while capturing the route and Cedar snapshots for one
    /// request. The publisher's write guard makes the pair process-local and
    /// atomic without holding configuration locks for the request lifetime.
    pub(crate) async fn configuration_snapshot_guard(
        &self,
    ) -> tokio::sync::OwnedRwLockReadGuard<()> {
        Arc::clone(&self.configuration_snapshot_lock)
            .read_owned()
            .await
    }

    pub(crate) async fn configuration_publication_guard(
        &self,
    ) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        Arc::clone(&self.configuration_snapshot_lock)
            .write_owned()
            .await
    }

    fn disable_authority_cache(&self) {
        if let Some(readiness) = &self.authority_readiness {
            readiness.request_bootstrap();
        }
    }

    fn disable_authority_cache_for_configuration(&self) {
        self.configuration_revision
            .store(CONFIG_REVISION_UNINITIALIZED, Ordering::SeqCst);
        #[cfg(feature = "redis-l2")]
        self.authority_config_epoch
            .store(CONFIG_EPOCH_INVALID, Ordering::SeqCst);
        self.disable_authority_cache();
    }

    async fn configuration_fence_is_current(&self, durable_revision: u64) -> Result<bool, ()> {
        let applied_revision = self.configuration_revision.load(Ordering::SeqCst);

        #[cfg(feature = "redis-l2")]
        if let Some(fence) = &self.authority_fence {
            let (shared_epoch, shared_revision) = match fence.read_configuration_state().await {
                Ok(state) => state,
                Err(error) => {
                    warn!(%error, "shared configuration state is unavailable");
                    return Ok(false);
                }
            };
            if shared_revision > durable_revision {
                error!(
                    shared_revision,
                    durable_revision, "shared configuration revision is ahead of durable state"
                );
                self.disable_authority_cache_for_configuration();
                return Err(());
            }
            let local_epoch = self.authority_config_epoch.load(Ordering::SeqCst);
            return Ok(applied_revision == durable_revision
                && shared_revision == durable_revision
                && local_epoch == shared_epoch);
        }

        Ok(applied_revision == durable_revision)
    }

    async fn configuration_fence_is_published_at(&self, revision: u64) -> bool {
        #[cfg(feature = "redis-l2")]
        if let Some(fence) = &self.authority_fence {
            return match fence.read_configuration_state().await {
                Ok((shared_epoch, shared_revision)) => {
                    shared_revision == revision
                        && self.authority_config_epoch.load(Ordering::SeqCst) == shared_epoch
                }
                Err(error) => {
                    warn!(%error, "shared configuration publication could not be confirmed");
                    false
                }
            };
        }

        true
    }

    /// Invalidate all cache entries. Called on config change.
    pub async fn invalidate_all(&self) {
        let publication_guard = self.configuration_publication_guard().await;
        let _ = self.invalidate_all_inner(None).await;
        drop(publication_guard);
        self.delete_l2_sessions().await;
    }

    async fn invalidate_all_at_revision(&self, config_revision: u64) -> bool {
        self.invalidate_all_inner(Some(config_revision)).await
    }

    async fn invalidate_all_inner(&self, config_revision: Option<u64>) -> bool {
        self.session_generation.fetch_add(1, Ordering::SeqCst);
        self.routes.invalidate_all();
        self.sessions.invalidate_all();
        self.kv.invalidate_all();

        let mut shared_epoch_ready = true;

        #[cfg(feature = "redis-l2")]
        {
            if let Some(fence) = &self.authority_fence {
                match fence.advance_configuration_epoch(config_revision).await {
                    Ok(current) => {
                        self.authority_config_epoch.store(current, Ordering::SeqCst);
                    }
                    Err(error) => {
                        shared_epoch_ready = false;
                        self.authority_config_epoch
                            .store(CONFIG_EPOCH_INVALID, Ordering::SeqCst);
                        self.disable_authority_cache();
                        warn!(%error, "shared configuration epoch invalidation failed");
                    }
                }
            }
        }

        info!("all caches invalidated");
        shared_epoch_ready
    }

    async fn delete_l2_sessions(&self) {
        #[cfg(feature = "redis-l2")]
        if let Some(ref con) = self.l2 {
            // The shared epoch already makes older entries unusable. Physical
            // deletion is best-effort work and must not extend the process-local
            // configuration publication lock seen by incoming requests.
            let mut con = con.clone();
            let patterns = [
                format!("{L2_PREFIX}:session:*"),
                format!("{L2_PREFIX}:v2:authority:*:session:*"),
            ];
            let mut deleted = 0;
            for pattern in patterns {
                match Self::scan_and_del(&mut con, &pattern).await {
                    Ok(count) => deleted += count,
                    Err(error) => warn!(%error, %pattern, "L2 Redis invalidation failed"),
                }
            }
            info!(deleted, "L2 Redis entries invalidated");
        }
    }

    #[cfg(feature = "redis-l2")]
    pub(crate) async fn invalidate_local_sessions(&self) {
        self.session_generation.fetch_add(1, Ordering::SeqCst);
        self.sessions.invalidate_all();
    }

    /// Invalidate the routes cache only.
    pub async fn invalidate_routes(&self) {
        self.routes.invalidate_all();
        info!("routes cache invalidated");
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
        let authority = self
            .authority_readiness
            .as_ref()
            .map(AuthorityReadiness::snapshot);
        CacheStats {
            routes_entry_count: self.routes.entry_count(),
            sessions_entry_count: self.sessions.entry_count(),
            kv_entry_count: self.kv.entry_count(),
            authority_cache: authority
                .as_ref()
                .map_or("not_configured", |snapshot| match snapshot.mode {
                    AuthorityCacheMode::Bootstrapping => "bootstrapping",
                    AuthorityCacheMode::Ready if self.authority_cache_ready() => "ready",
                    AuthorityCacheMode::Ready | AuthorityCacheMode::Bypass => "bypass",
                }),
            authority_applied_sequence: authority
                .as_ref()
                .and_then(|snapshot| snapshot.high_water.as_ref())
                .map(|water| water.outbox_sequence),
            authority_observed_high_water: authority
                .and_then(|snapshot| snapshot.observed_high_water),
        }
    }
}

/// Snapshot of cache entry counts.
#[derive(Debug, serde::Serialize)]
pub struct CacheStats {
    pub routes_entry_count: u64,
    pub sessions_entry_count: u64,
    pub kv_entry_count: u64,
    pub authority_cache: &'static str,
    pub authority_applied_sequence: Option<i64>,
    pub authority_observed_high_water: Option<i64>,
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
    match payload.split_once(':').map_or(payload, |(kind, _)| kind) {
        "routes" | "sites" => NotifyAction::Routes,
        "policies" => NotifyAction::Policies,
        _ => NotifyAction::All,
    }
}

fn notification_revision(payload: &str) -> Option<u64> {
    payload
        .split_once(':')
        .and_then(|(_, revision)| revision.parse().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigurationReconcile {
    Current,
    Advanced,
    Failed,
}

struct PreparedConfiguration {
    revision: u64,
    router: Option<crate::proxy::Router>,
    policies: Option<crate::authz::CedarBundle>,
    policy_count: Option<usize>,
}

async fn read_configuration_revision(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    let revision =
        sqlx::query_scalar::<_, i64>("SELECT revision FROM config_revision WHERE singleton = true")
            .fetch_one(pool)
            .await?;
    u64::try_from(revision).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

async fn prepare_configuration(
    pool: &sqlx::PgPool,
    cache: &GateCache,
    notified_revision: Option<u64>,
    router: &Option<(
        crate::proxy::SharedRouter,
        crate::config::SharedConfig,
        Arc<crate::db::Database>,
    )>,
    authz: &Option<(Arc<crate::authz::AuthzEngine>, Arc<crate::db::Database>)>,
    event_tx: &Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
) -> Result<Option<PreparedConfiguration>, ()> {
    let revision = match read_configuration_revision(pool).await {
        Ok(revision) => revision,
        Err(error) => {
            error!(%error, "durable configuration revision read failed");
            cache.disable_authority_cache_for_configuration();
            return Err(());
        }
    };
    if notified_revision.is_some_and(|notified| notified > revision) {
        error!(
            durable_revision = revision,
            notified_revision, "notification revision is ahead of durable configuration state"
        );
        cache.disable_authority_cache_for_configuration();
        return Err(());
    }

    let applied = cache.configuration_revision.load(Ordering::SeqCst);
    if applied != CONFIG_REVISION_UNINITIALIZED && applied > revision {
        error!(
            durable_revision = revision,
            applied_revision = applied,
            "durable configuration revision regressed"
        );
        cache.disable_authority_cache_for_configuration();
        return Err(());
    }
    match cache.configuration_fence_is_current(revision).await {
        Ok(true) => return Ok(None),
        Ok(false) => {}
        Err(()) => return Err(()),
    }

    let prepared_router = if let Some((_, shared_config, db)) = router {
        let config = shared_config.read().await.clone();
        let merged = if config.database.override_yaml {
            let db_routes = match db.load_routes().await {
                Ok(routes) => routes,
                Err(error) => {
                    warn!(%error, "failed to prepare DB routes — retaining last-good configuration");
                    cache.disable_authority_cache_for_configuration();
                    return Err(());
                }
            };
            crate::proxy::merge_routes(&config, &db_routes)
        } else {
            config.routes.clone()
        };
        let findings = config.agent_governance_lint_routes(&merged);
        for finding in &findings {
            warn!(
                route_id = %finding.route_id,
                "agent-governance (reload): {}",
                finding.reason.as_str()
            );
        }
        if reload_must_be_rejected(config.server.strict_agent_governance, findings.len()) {
            crate::metrics::record_governance_reload_rejected();
            warn!(
                findings = findings.len(),
                "agent-governance (reload, strict): retaining last-good configuration"
            );
            cache.disable_authority_cache_for_configuration();
            return Err(());
        }
        Some(crate::proxy::Router::from_config_with_routes(
            &config, merged,
        ))
    } else {
        None
    };

    let (prepared_policies, policy_count) = if let Some((authz, db)) = authz {
        let rows = match db.load_enabled_policies().await {
            Ok(rows) => rows,
            Err(error) => {
                error!(%error, "failed to prepare policies — retaining last-good configuration");
                if let Some(tx) = event_tx {
                    let _ = tx.send(crate::admin::AdminEvent::PolicyReloadError {
                        skipped_count: 0,
                        db_error: Some(error.to_string()),
                    });
                }
                cache.disable_authority_cache_for_configuration();
                return Err(());
            }
        };
        let records: Vec<crate::authz::PolicyRecord> = rows
            .into_iter()
            .map(crate::db::PolicyRow::into_record)
            .collect();
        let bundle = authz.prepare_records_lenient(&records);
        let count = bundle.policies().policies().count();
        (Some(bundle), Some(count))
    } else {
        (None, None)
    };

    let observed_after_prepare = match read_configuration_revision(pool).await {
        Ok(revision) => revision,
        Err(error) => {
            error!(%error, "durable configuration revision confirmation failed");
            cache.disable_authority_cache_for_configuration();
            return Err(());
        }
    };
    if observed_after_prepare != revision {
        warn!(
            prepared_revision = revision,
            durable_revision = observed_after_prepare,
            "configuration changed while candidates were prepared; retrying from the durable revision"
        );
        cache.disable_authority_cache_for_configuration();
        return Err(());
    }

    Ok(Some(PreparedConfiguration {
        revision,
        router: prepared_router,
        policies: prepared_policies,
        policy_count,
    }))
}

async fn reconcile_and_publish_configuration(
    pool: &sqlx::PgPool,
    cache: &GateCache,
    notified_revision: Option<u64>,
    router: &Option<(
        crate::proxy::SharedRouter,
        crate::config::SharedConfig,
        Arc<crate::db::Database>,
    )>,
    authz: &Option<(Arc<crate::authz::AuthzEngine>, Arc<crate::db::Database>)>,
    event_tx: &Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
) -> ConfigurationReconcile {
    let prepared = match prepare_configuration(
        pool,
        cache,
        notified_revision,
        router,
        authz,
        event_tx,
    )
    .await
    {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return ConfigurationReconcile::Current,
        Err(()) => return ConfigurationReconcile::Failed,
    };

    let mut publication_lock = match pool.begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            error!(%error, "failed to start configuration publication lock transaction");
            cache.disable_authority_cache_for_configuration();
            return ConfigurationReconcile::Failed;
        }
    };
    if let Err(error) = sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(crate::db::CONFIGURATION_PUBLICATION_LOCK)
        .execute(&mut *publication_lock)
        .await
    {
        error!(%error, "failed to acquire configuration publication lock");
        cache.disable_authority_cache_for_configuration();
        return ConfigurationReconcile::Failed;
    }
    let revision_under_lock = match sqlx::query_scalar::<_, i64>(
        "SELECT revision FROM config_revision WHERE singleton = true",
    )
    .fetch_one(&mut *publication_lock)
    .await
    {
        Ok(revision) => match u64::try_from(revision) {
            Ok(revision) => revision,
            Err(error) => {
                error!(%error, "configuration revision under publication lock is invalid");
                cache.disable_authority_cache_for_configuration();
                return ConfigurationReconcile::Failed;
            }
        },
        Err(error) => {
            error!(%error, "failed to confirm configuration revision under publication lock");
            cache.disable_authority_cache_for_configuration();
            return ConfigurationReconcile::Failed;
        }
    };
    if revision_under_lock != prepared.revision {
        warn!(
            prepared_revision = prepared.revision,
            durable_revision = revision_under_lock,
            "configuration changed before publication lock; retrying"
        );
        cache.disable_authority_cache_for_configuration();
        return ConfigurationReconcile::Failed;
    }

    let outcome = publish_prepared_configuration(
        cache,
        router.as_ref().map(|(shared_router, _, _)| shared_router),
        authz.as_ref().map(|(authz, _)| authz),
        event_tx,
        prepared,
    )
    .await;
    if outcome == ConfigurationReconcile::Failed {
        cache.disable_authority_cache_for_configuration();
        return outcome;
    }

    if let Err(error) = publication_lock.commit().await {
        error!(%error, "failed to release configuration publication lock cleanly");
        cache.disable_authority_cache_for_configuration();
        return ConfigurationReconcile::Failed;
    }
    outcome
}

/// Publish the durable configuration revision through the same fenced path used
/// by the background listener. Admin mutation handlers await this function so
/// they cannot expose a policy bundle outside the shared Redis epoch or paired
/// route publication.
pub async fn reconcile_configuration_after_mutation(
    pool: &sqlx::PgPool,
    cache: &GateCache,
    router: Option<(
        crate::proxy::SharedRouter,
        crate::config::SharedConfig,
        Arc<crate::db::Database>,
    )>,
    authz: Option<(Arc<crate::authz::AuthzEngine>, Arc<crate::db::Database>)>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
) -> bool {
    !matches!(
        reconcile_and_publish_configuration(pool, cache, None, &router, &authz, &event_tx,).await,
        ConfigurationReconcile::Failed
    )
}

/// Install the first database-backed route and Cedar pair before HTTP listeners
/// bind. The same stable-revision reads, PostgreSQL advisory lock, shared Redis
/// epoch, and process-local publication guard are used for startup and reloads.
pub async fn initialize_configuration(
    pool: &sqlx::PgPool,
    cache: &GateCache,
    router: Option<(
        crate::proxy::SharedRouter,
        crate::config::SharedConfig,
        Arc<crate::db::Database>,
    )>,
    authz: Option<(Arc<crate::authz::AuthzEngine>, Arc<crate::db::Database>)>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
) -> bool {
    !matches!(
        reconcile_and_publish_configuration(pool, cache, None, &router, &authz, &event_tx).await,
        ConfigurationReconcile::Failed
    )
}

async fn publish_prepared_configuration(
    cache: &GateCache,
    router: Option<&crate::proxy::SharedRouter>,
    authz: Option<&Arc<crate::authz::AuthzEngine>>,
    event_tx: &Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
    prepared: PreparedConfiguration,
) -> ConfigurationReconcile {
    // Requests capture the route and Cedar snapshots under the matching read
    // guard. Holding the write guard from before the shared epoch advances
    // through both swaps and the installed revision makes the local triple one
    // observable configuration revision.
    let publication_guard = cache.configuration_publication_guard().await;
    if !cache.invalidate_all_at_revision(prepared.revision).await {
        return ConfigurationReconcile::Failed;
    }

    if !cache
        .configuration_fence_is_published_at(prepared.revision)
        .await
    {
        return ConfigurationReconcile::Failed;
    }

    if let (Some(shared_router), Some(prepared_router)) = (router, prepared.router) {
        let route_count = prepared_router.route_count();
        *shared_router.write().await = prepared_router;
        info!(
            route_count,
            "router published from stable configuration revision"
        );
    }
    if let (Some(authz), Some(prepared_policies)) = (authz, prepared.policies) {
        authz.install_prepared_bundle(prepared_policies);
        if let (Some(tx), Some(policy_count)) = (event_tx, prepared.policy_count) {
            let _ = tx.send(crate::admin::AdminEvent::PolicyReloadOk { policy_count });
        }
    }
    cache
        .configuration_revision
        .store(prepared.revision, Ordering::SeqCst);
    drop(publication_guard);
    cache.delete_l2_sessions().await;
    ConfigurationReconcile::Advanced
}

#[cfg(test)]
async fn reconcile_configuration_revision(
    pool: &sqlx::PgPool,
    cache: &GateCache,
    notified_revision: Option<u64>,
) -> ConfigurationReconcile {
    let router = None;
    let authz = None;
    let event_tx = None;
    reconcile_and_publish_configuration(pool, cache, notified_revision, &router, &authz, &event_tx)
        .await
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
///
/// When `event_tx` is `Some`, an `AdminEvent` is broadcast after every reload
/// attempt so admin SSE subscribers receive real-time reload status.
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
    event_tx: Option<tokio::sync::broadcast::Sender<crate::admin::AdminEvent>>,
) {
    tokio::spawn(async move {
        let mut retry = tokio::time::interval(Duration::from_secs(1));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        retry.tick().await;
        'supervisor: loop {
            let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(listener) => listener,
                Err(error) => {
                    warn!(%error, "failed to create PG listener; retrying");
                    cache.disable_authority_cache_for_configuration();
                    retry.tick().await;
                    continue;
                }
            };
            if let Err(error) = listener.listen(&channel).await {
                warn!(%error, %channel, "failed to LISTEN on PG channel; retrying");
                cache.disable_authority_cache_for_configuration();
                retry.tick().await;
                continue;
            }
            info!(channel = %channel, "listening for cache invalidation notifications");
            let _ = reconcile_and_publish_configuration(
                &pool, &cache, None, &router, &authz, &event_tx,
            )
            .await;

            loop {
                tokio::select! {
                    notification = listener.recv() => match notification {
                        Ok(notification) => {
                            let payload = notification.payload();
                            let action = classify_notification(payload);
                            let revision = notification_revision(payload);
                            info!(channel = %channel, payload = %payload, ?action, "cache invalidation notification received");

                            let _ = reconcile_and_publish_configuration(
                                &pool,
                                &cache,
                                revision,
                                &router,
                                &authz,
                                &event_tx,
                            ).await;
                        }
                        Err(error) => {
                            warn!(%error, "LISTEN/NOTIFY error; reconnecting");
                            cache.disable_authority_cache_for_configuration();
                            retry.tick().await;
                            continue 'supervisor;
                        }
                    },
                    _ = retry.tick() => {
                        let _ = reconcile_and_publish_configuration(
                            &pool,
                            &cache,
                            None,
                            &router,
                            &authz,
                            &event_tx,
                        ).await;
                    },
                }
            }
        }
    });
}

/// Decide whether a route hot-reload must be rejected (last-good retained).
///
/// Fail-closed posture for a LIVE process (which cannot refuse-to-start): a reload
/// is rejected only under `strict_agent_governance` AND when the merged set has at
/// least one governance finding. Pure, so the reload policy is unit-testable
/// without a database.
fn reload_must_be_rejected(strict: bool, finding_count: usize) -> bool {
    strict && finding_count > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authz::{AuthzDecision, AuthzEngine, PolicyRecord, PrincipalKind},
        config::types::{
            CacheConfig, GateConfig, HooksConfig, RouteConfig, RouteMatch, SiteConfig, StreamConfig,
        },
        proxy::Router,
    };

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

    #[tokio::test]
    async fn request_snapshot_cannot_observe_partial_route_policy_publication() {
        fn route(id: &str) -> RouteConfig {
            RouteConfig {
                id: id.to_owned(),
                site: "atomic-site".to_owned(),
                route_match: RouteMatch {
                    path: "/atomic".to_owned(),
                    methods: vec!["GET".to_owned()],
                    host: None,
                },
                upstream: Some("http://127.0.0.1:9".to_owned()),
                auth: None,
                hooks: HooksConfig::default(),
                stream: StreamConfig::default(),
                priority: 0,
                enabled: true,
            }
        }

        let config = GateConfig {
            sites: vec![SiteConfig {
                id: "atomic-site".to_owned(),
                domains: Vec::new(),
                default_auth: None,
                default_upstream: None,
            }],
            ..GateConfig::default()
        };
        let shared_router = Arc::new(tokio::sync::RwLock::new(Router::from_config_with_routes(
            &config,
            vec![route("old-route")],
        )));
        let authz = Arc::new(AuthzEngine::empty());
        let cache = GateCache::from_config(&CacheConfig::default());
        let permit = authz.prepare_records_lenient(&[PolicyRecord {
            id: "permit-new".to_owned(),
            policy_text: "permit(principal, action, resource);".to_owned(),
            schema_json: None,
            entities_json: None,
        }]);
        let prepared = PreparedConfiguration {
            revision: 1,
            router: Some(Router::from_config_with_routes(
                &config,
                vec![route("new-route")],
            )),
            policies: Some(permit),
            policy_count: Some(1),
        };
        let event_tx = None;

        let request_guard = cache.configuration_snapshot_guard().await;
        let publisher_cache = cache.clone();
        let publisher_router = Arc::clone(&shared_router);
        let publisher_authz = Arc::clone(&authz);
        let publisher = tokio::spawn(async move {
            publish_prepared_configuration(
                &publisher_cache,
                Some(&publisher_router),
                Some(&publisher_authz),
                &event_tx,
                prepared,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(
            !publisher.is_finished(),
            "publication must wait while a request captures its route and policy pair"
        );
        let old_route = shared_router
            .read()
            .await
            .match_route("", "/atomic", "GET")
            .unwrap()
            .config
            .id
            .clone();
        let old_authz = Arc::new(authz.pinned_snapshot());
        drop(request_guard);

        assert_eq!(publisher.await.unwrap(), ConfigurationReconcile::Advanced);
        let new_guard = cache.configuration_snapshot_guard().await;
        let new_route = shared_router
            .read()
            .await
            .match_route("", "/atomic", "GET")
            .unwrap()
            .config
            .id
            .clone();
        let new_authz = Arc::new(authz.pinned_snapshot());
        drop(new_guard);

        assert_eq!(old_route, "old-route");
        assert_eq!(new_route, "new-route");
        assert_eq!(
            old_authz.authorize_as(PrincipalKind::User, "u", "invoke", "r", &Value::Null),
            AuthzDecision::Deny
        );
        assert_eq!(
            new_authz.authorize_as(PrincipalKind::User, "u", "invoke", "r", &Value::Null),
            AuthzDecision::Allow
        );
    }

    // ── C1: NOTIFY payload dispatch (the arm the listener selects) ───────────

    #[test]
    fn classify_routes_and_sites_map_to_routes() {
        assert_eq!(classify_notification("routes"), NotifyAction::Routes);
        assert_eq!(classify_notification("sites"), NotifyAction::Routes);
        assert_eq!(classify_notification("routes:42"), NotifyAction::Routes);
        assert_eq!(notification_revision("routes:42"), Some(42));
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
        assert_eq!(notification_revision("routes:invalid"), None);
        assert_eq!(notification_revision("routes"), None);
    }

    // ── reload governance decision (fail-closed for a live process) ──────────

    #[test]
    fn reload_rejected_only_under_strict_with_findings() {
        // Strict + findings → reject (retain last-good); a live process can't bail.
        assert!(reload_must_be_rejected(true, 1));
        assert!(reload_must_be_rejected(true, 5));
    }

    #[test]
    fn reload_applies_when_not_strict_even_with_findings() {
        // Non-strict → advisory only: warn but still apply the reload.
        assert!(!reload_must_be_rejected(false, 3));
    }

    #[test]
    fn reload_applies_when_strict_but_clean() {
        // Strict but no findings → apply normally.
        assert!(!reload_must_be_rejected(true, 0));
        assert!(!reload_must_be_rejected(false, 0));
    }
}
