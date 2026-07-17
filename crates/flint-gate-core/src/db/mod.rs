/// Database access layer — wraps `sqlx::PgPool` with Flint Gate schema operations.
///
/// Schema is applied via `migrate()` at startup. All mutations emit a Postgres
/// NOTIFY on `flintgate_config_changed` so that other instances invalidate caches.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tracing::{debug, info};
use uuid::Uuid;

/// SHA-256 of `input`, hex-encoded. Used to store API-key hashes so raw secrets
/// are never persisted. (Client secrets now use [`SecretHash`] — bcrypt.)
fn sha256_hex(input: &str) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(input.as_bytes());
    hex::encode(h.finalize())
}

/// bcrypt work factor for client-secret hashing. Client secrets are 256-bit
/// CSPRNG tokens, so the default cost is comfortably sufficient.
const BCRYPT_COST: u32 = bcrypt::DEFAULT_COST;

/// Password/secret hashing for OAuth client secrets.
///
/// New secrets are hashed with **bcrypt** (per-hash salt, tunable work factor).
/// Verification **format-sniffs** the stored hash so pre-existing unsalted
/// SHA-256 rows keep working: a bcrypt hash (`$2b$…` / `$2a$…` / `$2y$…`)
/// verifies via `bcrypt::verify`; anything else is treated as a legacy 64-hex
/// SHA-256 hash and compared by re-hashing. A legacy verify signals
/// [`SecretHash::needs_rehash`] so the caller can transparently upgrade the row.
struct SecretHash;

impl SecretHash {
    /// bcrypt silently truncates its input at 72 bytes. Every secret we create
    /// is 64 hex chars, so this is never hit in practice — but we enforce the
    /// bound explicitly so a future path that hashes a longer, caller-supplied
    /// secret fails loudly rather than silently colliding on a 72-byte prefix.
    const MAX_SECRET_LEN: usize = 72;

    /// Hash a raw secret with bcrypt for storage. Rejects an over-length secret
    /// (bcrypt would otherwise truncate at 72 bytes without error).
    fn hash(raw: &str) -> anyhow::Result<String> {
        if raw.len() > Self::MAX_SECRET_LEN {
            anyhow::bail!(
                "client secret exceeds {} bytes (bcrypt would truncate)",
                Self::MAX_SECRET_LEN
            );
        }
        bcrypt::hash(raw, BCRYPT_COST).context("hashing client secret")
    }

    /// True when the stored hash is a modern bcrypt hash.
    fn is_bcrypt(stored: &str) -> bool {
        stored.starts_with("$2b$") || stored.starts_with("$2a$") || stored.starts_with("$2y$")
    }

    /// Verify a raw secret against a stored hash (bcrypt or legacy SHA-256).
    /// Never errors on a bad hash format — an unverifiable hash is simply `false`.
    fn verify(raw: &str, stored: &str) -> bool {
        if Self::is_bcrypt(stored) {
            bcrypt::verify(raw, stored).unwrap_or(false)
        } else {
            // Legacy unsalted SHA-256 (64 hex chars). Constant-ish: both sides
            // are fixed-length hex compared byte-for-byte.
            sha256_hex(raw) == stored
        }
    }

    /// Whether a successfully-verified stored hash should be upgraded to bcrypt.
    fn needs_rehash(stored: &str) -> bool {
        !Self::is_bcrypt(stored)
    }
}

/// Database access wrapper.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

/// Usage summary for a time window.
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub total_tokens: i64,
    pub total_requests: i64,
    pub total_duration_ms: i64,
    pub avg_tokens_per_request: f64,
    pub avg_duration_ms: f64,
}

/// One point in a token/time usage time series.
#[derive(Debug, Clone, Serialize)]
pub struct UsageTimeSeriesPoint {
    pub bucket: String,
    pub tokens: i64,
    pub requests: i64,
}

/// Token usage grouped by route.
#[derive(Debug, Clone, Serialize)]
pub struct RouteUsage {
    pub route_id: String,
    pub tokens: i64,
    pub requests: i64,
}

/// Token usage grouped by user.
#[derive(Debug, Clone, Serialize)]
pub struct UserUsage {
    pub user_id: String,
    pub tokens: i64,
    pub requests: i64,
}

impl Database {
    /// Return aggregate token/request/duration statistics for a time window.
    /// Both bounds are optional; absence means "all time".
    pub async fn usage_summary(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<UsageSummary> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(tokens), 0) AS total_tokens, \
             COUNT(*) AS total_requests, \
             COALESCE(SUM(duration_ms), 0) AS total_duration_ms \
             FROM usage_events \
             WHERE ($1::timestamptz IS NULL OR created_at >= $1) \
               AND ($2::timestamptz IS NULL OR created_at <= $2)",
        )
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await
        .context("summarizing usage events")?;

        let total_tokens: i64 = row.try_get("total_tokens")?;
        let total_requests: i64 = row.try_get("total_requests")?;
        let total_duration_ms: i64 = row.try_get("total_duration_ms")?;

        let avg_tokens_per_request = if total_requests > 0 {
            total_tokens as f64 / total_requests as f64
        } else {
            0.0
        };
        let avg_duration_ms = if total_requests > 0 {
            total_duration_ms as f64 / total_requests as f64
        } else {
            0.0
        };

        Ok(UsageSummary {
            total_tokens,
            total_requests,
            total_duration_ms,
            avg_tokens_per_request,
            avg_duration_ms,
        })
    }

    /// Return a token/time time series bucketed by the requested interval
    /// (`hour` or `day`). Empty buckets are not emitted — clients can fill gaps
    /// if they need a regular grid.
    pub async fn usage_timeseries(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        interval: &str,
    ) -> Result<Vec<UsageTimeSeriesPoint>> {
        let trunc = match interval {
            "hour" => "hour",
            "day" => "day",
            _ => "day",
        };
        let sql = format!(
            "SELECT date_trunc('{}', created_at) AS bucket, \
             COALESCE(SUM(tokens), 0) AS tokens, \
             COUNT(*) AS requests \
             FROM usage_events \
             WHERE ($1::timestamptz IS NULL OR created_at >= $1) \
               AND ($2::timestamptz IS NULL OR created_at <= $2) \
             GROUP BY bucket \
             ORDER BY bucket ASC",
            trunc
        );
        let rows = sqlx::query(&sql)
            .bind(since)
            .bind(until)
            .fetch_all(&self.pool)
            .await
            .context("querying usage timeseries")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let bucket: DateTime<Utc> = r.try_get("bucket")?;
            out.push(UsageTimeSeriesPoint {
                bucket: bucket.to_rfc3339(),
                tokens: r.try_get("tokens")?,
                requests: r.try_get("requests")?,
            });
        }
        Ok(out)
    }

    /// Top routes by token usage for a time window.
    pub async fn usage_by_route(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<RouteUsage>> {
        let rows = sqlx::query(
            "SELECT route_id, COALESCE(SUM(tokens), 0) AS tokens, COUNT(*) AS requests \
             FROM usage_events \
             WHERE ($1::timestamptz IS NULL OR created_at >= $1) \
               AND ($2::timestamptz IS NULL OR created_at <= $2) \
             GROUP BY route_id \
             ORDER BY tokens DESC \
             LIMIT $3",
        )
        .bind(since)
        .bind(until)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("querying usage by route")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(RouteUsage {
                route_id: r.try_get("route_id")?,
                tokens: r.try_get("tokens")?,
                requests: r.try_get("requests")?,
            });
        }
        Ok(out)
    }

    /// Top users by token usage for a time window.
    pub async fn usage_by_user(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<UserUsage>> {
        let rows = sqlx::query(
            "SELECT user_id, COALESCE(SUM(tokens), 0) AS tokens, COUNT(*) AS requests \
             FROM usage_events \
             WHERE ($1::timestamptz IS NULL OR created_at >= $1) \
               AND ($2::timestamptz IS NULL OR created_at <= $2) \
             GROUP BY user_id \
             ORDER BY tokens DESC \
             LIMIT $3",
        )
        .bind(since)
        .bind(until)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("querying usage by user")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(UserUsage {
                user_id: r.try_get("user_id")?,
                tokens: r.try_get("tokens")?,
                requests: r.try_get("requests")?,
            });
        }
        Ok(out)
    }

    /// Connect to Postgres and return a `Database`.
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await
            .context("connecting to Postgres")?;
        Ok(Self { pool })
    }

    /// Return the underlying pool (e.g. for LISTEN/NOTIFY).
    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// Run all pending sqlx migrations from `migrations/`.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!()
            .run(&self.pool)
            .await
            .context("running database migrations")?;
        info!("database migrations applied");
        Ok(())
    }

    /// Load all enabled routes from the database.
    pub async fn load_routes(&self) -> Result<Vec<DbRoute>> {
        let rows = sqlx::query("SELECT id, config, priority, enabled FROM gate_routes WHERE enabled = true ORDER BY priority DESC")
            .fetch_all(&self.pool)
            .await
            .context("loading routes")?;

        let mut routes = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let config: serde_json::Value = row.try_get("config")?;
            let priority: i32 = row.try_get("priority")?;
            let enabled: bool = row.try_get("enabled")?;
            routes.push(DbRoute {
                id,
                config,
                priority,
                enabled,
            });
        }
        Ok(routes)
    }

    /// Load a single route by ID.
    pub async fn get_route(&self, id: &str) -> Result<Option<DbRoute>> {
        let row =
            sqlx::query("SELECT id, config, priority, enabled FROM gate_routes WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .context("loading route by id")?;

        match row {
            None => Ok(None),
            Some(r) => Ok(Some(DbRoute {
                id: r.try_get("id")?,
                config: r.try_get("config")?,
                priority: r.try_get("priority")?,
                enabled: r.try_get("enabled")?,
            })),
        }
    }

    /// Upsert a route configuration.
    pub async fn upsert_route(
        &self,
        id: &str,
        config: &serde_json::Value,
        priority: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO gate_routes (id, config, priority, updated_at) VALUES ($1, $2, $3, NOW())
             ON CONFLICT (id) DO UPDATE SET config = $2, priority = $3, updated_at = NOW()",
        )
        .bind(id)
        .bind(config)
        .bind(priority)
        .execute(&self.pool)
        .await
        .context("upserting route")?;

        self.notify("routes").await?;
        Ok(())
    }

    /// Delete a route by ID.
    pub async fn delete_route(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM gate_routes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("deleting route")?;

        if result.rows_affected() > 0 {
            self.notify("routes").await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Log a usage event (token metering, billing).
    pub async fn log_usage(&self, event: &UsageEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO usage_events (id, request_id, user_id, route_id, tokens, duration_ms, metadata)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        )
        .bind(event.id)
        .bind(&event.request_id)
        .bind(&event.user_id)
        .bind(&event.route_id)
        .bind(event.tokens)
        .bind(event.duration_ms)
        .bind(&event.metadata)
        .execute(&self.pool)
        .await
        .context("logging usage event")?;

        debug!(request_id = %event.request_id, tokens = event.tokens, "usage event logged");
        Ok(())
    }

    /// Validate an API key by comparing its SHA-256 hash against the database.
    ///
    /// Returns the associated `ApiKeyRecord` if found and active.
    pub async fn validate_api_key(&self, key_hash: &str) -> Result<Option<ApiKeyRecord>> {
        let row = sqlx::query(
            "SELECT id, client_id, role, principal_type, scopes, expires_at FROM api_keys
             WHERE key_hash = $1 AND active = true AND (expires_at IS NULL OR expires_at > NOW())",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await
        .context("validating API key")?;

        match row {
            None => Ok(None),
            Some(r) => {
                let id: Uuid = r.try_get("id")?;
                let client_id: String = r.try_get("client_id")?;
                let role: String = r.try_get("role")?;
                let principal_type: String = r.try_get("principal_type")?;
                let scopes: serde_json::Value = r.try_get("scopes")?;
                let expires_at: Option<DateTime<Utc>> = r.try_get("expires_at")?;

                let scopes: Vec<String> = scopes
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                Ok(Some(ApiKeyRecord {
                    id,
                    client_id,
                    role,
                    principal_type,
                    scopes,
                    expires_at,
                }))
            }
        }
    }

    /// Create a new API key. Returns `(id, raw_key)` — the raw key is only
    /// returned once and must be presented to the caller immediately.
    pub async fn create_api_key(
        &self,
        client_id: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(Uuid, String)> {
        use rand::Rng;

        // Generate a 32-byte random key and encode as hex (64-char string).
        let raw_bytes: [u8; 32] = rand::thread_rng().gen();
        let raw_key = hex::encode(raw_bytes);
        let key_hash = sha256_hex(&raw_key);

        let scopes_json = serde_json::to_value(scopes).context("serializing scopes")?;

        let row = sqlx::query(
            "INSERT INTO api_keys (key_hash, client_id, scopes, expires_at)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(&key_hash)
        .bind(client_id)
        .bind(&scopes_json)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .context("creating API key")?;

        let id: Uuid = row.try_get("id")?;
        info!(client_id, %id, "API key created");
        Ok((id, raw_key))
    }

    /// List all active API keys (metadata only — never returns key hashes).
    pub async fn list_api_keys(&self) -> Result<Vec<ApiKeyRecord>> {
        let rows = sqlx::query(
            "SELECT id, client_id, role, principal_type, scopes, expires_at FROM api_keys WHERE active = true ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing API keys")?;

        let mut keys = Vec::with_capacity(rows.len());
        for r in rows {
            let id: Uuid = r.try_get("id")?;
            let client_id: String = r.try_get("client_id")?;
            let role: String = r.try_get("role")?;
            let principal_type: String = r.try_get("principal_type")?;
            let scopes: serde_json::Value = r.try_get("scopes")?;
            let expires_at: Option<DateTime<Utc>> = r.try_get("expires_at")?;
            let scopes: Vec<String> = scopes
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            keys.push(ApiKeyRecord {
                id,
                client_id,
                role,
                principal_type,
                scopes,
                expires_at,
            });
        }
        Ok(keys)
    }

    /// Revoke (soft-delete) an API key by ID. Returns `false` if not found.
    pub async fn revoke_api_key(&self, id: Uuid) -> Result<bool> {
        let result =
            sqlx::query("UPDATE api_keys SET active = false WHERE id = $1 AND active = true")
                .bind(id)
                .execute(&self.pool)
                .await
                .context("revoking API key")?;
        Ok(result.rows_affected() > 0)
    }

    // ── OAuth clients (client_credentials grant) ─────────────────────────────

    /// Create an OAuth client. Returns `(id, raw_secret)`; the raw secret is
    /// shown ONCE and only its SHA-256 hash is stored (mirrors `create_api_key`).
    pub async fn create_oauth_client(
        &self,
        client_id: &str,
        scopes: &[String],
        audience: Option<&str>,
    ) -> Result<(Uuid, String)> {
        use rand::Rng;
        // 256-bit CSPRNG secret — this is the ONLY path that creates a client
        // secret, so every stored secret is high-entropy + bcrypt-hashed.
        let raw_bytes: [u8; 32] = rand::thread_rng().gen();
        let raw_secret = hex::encode(raw_bytes);
        let secret_hash = SecretHash::hash(&raw_secret)?;
        let scopes_json = serde_json::to_value(scopes).context("serializing scopes")?;

        let row = sqlx::query(
            "INSERT INTO oauth_clients (client_id, secret_hash, scopes, audience)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(client_id)
        .bind(&secret_hash)
        .bind(&scopes_json)
        .bind(audience)
        .fetch_one(&self.pool)
        .await
        .context("creating OAuth client")?;

        let id: Uuid = row.try_get("id")?;
        info!(client_id, %id, "OAuth client created");
        Ok((id, raw_secret))
    }

    /// Verify a `client_id` + `client_secret` pair. Returns the client record on
    /// success, `None` on any mismatch (unknown client, wrong secret, inactive).
    ///
    /// The row is fetched by `client_id`, then the presented secret is verified
    /// against the stored hash via [`SecretHash`] (bcrypt, or legacy SHA-256 with
    /// a transparent upgrade). The raw secret is never persisted or compared in
    /// the clear.
    pub async fn verify_client_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Option<OAuthClientRecord>> {
        // Fetch by client_id (the secret hash is per-hash-salted with bcrypt, so
        // a `WHERE secret_hash = $2` lookup is impossible) then KDF-verify.
        let row = sqlx::query(
            "SELECT id, client_id, secret_hash, scopes, audience FROM oauth_clients
             WHERE client_id = $1 AND active = true",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await
        .context("verifying client credentials")?;

        let Some(r) = row else { return Ok(None) };
        let stored_hash: String = r.try_get("secret_hash")?;

        if !SecretHash::verify(client_secret, &stored_hash) {
            return Ok(None);
        }

        let id: Uuid = r.try_get("id")?;

        // Transparently upgrade a legacy (SHA-256) hash to bcrypt on a successful
        // verify — best-effort: a failed upgrade never fails the auth.
        if SecretHash::needs_rehash(&stored_hash) {
            if let Ok(new_hash) = SecretHash::hash(client_secret) {
                if let Err(e) = sqlx::query(
                    "UPDATE oauth_clients SET secret_hash = $1 WHERE id = $2",
                )
                .bind(&new_hash)
                .bind(id)
                .execute(&self.pool)
                .await
                {
                    tracing::warn!(error = %e, client_id, "client secret re-hash to bcrypt failed (ignored)");
                } else {
                    info!(client_id, "client secret upgraded to bcrypt");
                }
            }
        }

        let scopes: serde_json::Value = r.try_get("scopes")?;
        let scopes: Vec<String> = scopes
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(OAuthClientRecord {
            id,
            client_id: r.try_get("client_id")?,
            scopes,
            audience: r.try_get("audience")?,
        }))
    }

    // ── NHI lifecycle (agent / service identities) ───────────────────────────

    /// Issue (register) a non-human identity. `kind` is `"agent"` or `"service"`.
    /// Idempotent on the id (upsert keeps it active and updates the label).
    /// Insert an NHI-lifecycle audit row inside an open transaction. The row is
    /// an administrative `allow` decision distinguished by its `action`
    /// (`nhi_issue`/`nhi_rotate`/`nhi_revoke`), so it commits atomically with the
    /// status change — audited-before-effect.
    async fn insert_nhi_audit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        action: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO authz_audit \
             (id, request_id, principal, action, resource, decision, reason, context) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::new_v4())
        .bind(Option::<String>::None)
        .bind(id)
        .bind(action)
        .bind("agent_identity")
        .bind(AuthzAuditDecision::Allow.as_str())
        .bind(Some(format!("nhi {action}")))
        .bind(Some(serde_json::json!({ "agent_id": id })))
        .execute(&mut **tx)
        .await
        .context("writing NHI lifecycle audit row")?;
        Ok(())
    }

    pub async fn issue_agent_identity(
        &self,
        id: &str,
        kind: &str,
        label: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin issue txn")?;
        sqlx::query(
            "INSERT INTO agent_identities (id, kind, status, label)
             VALUES ($1, $2, 'active', $3)
             ON CONFLICT (id) DO UPDATE SET kind = EXCLUDED.kind, label = EXCLUDED.label, status = 'active'",
        )
        .bind(id)
        .bind(kind)
        .bind(label)
        .execute(&mut *tx)
        .await
        .context("issuing agent identity")?;
        Self::insert_nhi_audit(&mut tx, id, "nhi_issue").await?;
        tx.commit().await.context("commit issue txn")?;
        info!(id, kind, "agent identity issued");
        Ok(())
    }

    /// Mark an identity rotated (stamps `rotated_at`). Caller rotates the
    /// underlying credential separately (client secret / signing key).
    pub async fn rotate_agent_identity(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await.context("begin rotate txn")?;
        let r = sqlx::query(
            "UPDATE agent_identities SET rotated_at = NOW() WHERE id = $1 AND status = 'active'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("rotating agent identity")?;
        let changed = r.rows_affected() > 0;
        // Only audit (and commit meaningfully) when a row was actually rotated.
        if changed {
            Self::insert_nhi_audit(&mut tx, id, "nhi_rotate").await?;
        }
        tx.commit().await.context("commit rotate txn")?;
        Ok(changed)
    }

    /// Revoke a non-human identity. After this returns, [`Self::is_agent_revoked`]
    /// reports it revoked, so the next authorize denies it (fail-closed).
    pub async fn revoke_agent_identity(&self, id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await.context("begin revoke txn")?;
        let r = sqlx::query(
            "UPDATE agent_identities SET status = 'revoked' WHERE id = $1 AND status = 'active'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("revoking agent identity")?;
        let changed = r.rows_affected() > 0;
        // The revoke and its audit row commit together — audited-before-effect.
        // If the audit insert fails, the whole revoke rolls back.
        if changed {
            Self::insert_nhi_audit(&mut tx, id, "nhi_revoke").await?;
        }
        tx.commit().await.context("commit revoke txn")?;
        if changed {
            info!(id, "agent identity revoked");
        }
        Ok(changed)
    }

    /// Whether a non-human identity is revoked. An id that was never issued is
    /// treated as **not revoked** here (unknown ids are governed by policy, not
    /// the revocation list); only an explicit `revoked` row denies.
    pub async fn is_agent_revoked(&self, id: &str) -> Result<bool> {
        let row = sqlx::query(
            "SELECT status FROM agent_identities WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("checking agent identity revocation")?;
        Ok(row
            .map(|r| r.try_get::<String, _>("status").map(|s| s == "revoked"))
            .transpose()?
            .unwrap_or(false))
    }

    /// List all non-human identities (newest first).
    pub async fn list_agent_identities(&self) -> Result<Vec<AgentIdentityRecord>> {
        let rows = sqlx::query(
            "SELECT id, kind, status, label, rotated_at, created_at
             FROM agent_identities ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing agent identities")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(AgentIdentityRecord {
                id: r.try_get("id")?,
                kind: r.try_get("kind")?,
                status: r.try_get("status")?,
                label: r.try_get("label")?,
                rotated_at: r.try_get("rotated_at")?,
                created_at: r.try_get("created_at")?,
            });
        }
        Ok(out)
    }

    /// Return the lifetime token total for a user (for `usage_budget` lookup).
    pub async fn get_user_token_total(&self, user_id: &str) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(tokens), 0) AS total FROM usage_events WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .context("querying user token total")?;
        let total: i64 = row.try_get("total")?;
        Ok(total)
    }

    /// Return the token total for a user within a rolling time window.
    ///
    /// `interval` must be a Postgres interval literal (e.g. `"1 hour"`). Only
    /// `usage_events` rows newer than `now() - interval` are summed. This is the
    /// fallback path for windowed token budgets when Redis (`redis-l2`) is not
    /// enabled. The `created_at TIMESTAMPTZ` column is the event timestamp.
    pub async fn get_user_token_total_windowed(
        &self,
        user_id: &str,
        interval: &str,
    ) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(tokens), 0) AS total FROM usage_events \
             WHERE user_id = $1 AND created_at > now() - $2::interval",
        )
        .bind(user_id)
        .bind(interval)
        .fetch_one(&self.pool)
        .await
        .context("querying windowed user token total")?;
        let total: i64 = row.try_get("total")?;
        Ok(total)
    }

    /// Send a Postgres NOTIFY on the invalidation channel.
    async fn notify(&self, payload: &str) -> Result<()> {
        sqlx::query("SELECT pg_notify('flintgate_config_changed', $1)")
            .bind(payload)
            .execute(&self.pool)
            .await
            .context("sending pg_notify")?;
        Ok(())
    }

    /// Get the active JWT signing key from the database.
    pub async fn get_active_signing_key(&self) -> Result<Option<JwtSigningKey>> {
        let row = sqlx::query(
            "SELECT id, algorithm, public_key, private_key, active, created_at \
             FROM jwt_signing_keys WHERE active = true ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .context("querying active signing key")?;

        match row {
            None => Ok(None),
            Some(r) => Ok(Some(JwtSigningKey {
                id: r.try_get("id")?,
                algorithm: r.try_get("algorithm")?,
                public_key: r.try_get("public_key")?,
                private_key: r.try_get("private_key")?,
                active: r.try_get("active")?,
                created_at: r.try_get("created_at")?,
            })),
        }
    }

    /// List all JWT signing keys (never returns private_key to caller).
    pub async fn list_signing_keys(&self) -> Result<Vec<JwtSigningKeyPublic>> {
        let rows = sqlx::query(
            "SELECT id, algorithm, public_key, active, created_at \
             FROM jwt_signing_keys ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing signing keys")?;

        let mut keys = Vec::with_capacity(rows.len());
        for r in rows {
            keys.push(JwtSigningKeyPublic {
                id: r.try_get("id")?,
                algorithm: r.try_get("algorithm")?,
                public_key: r.try_get("public_key")?,
                active: r.try_get("active")?,
                created_at: r.try_get("created_at")?,
            });
        }
        Ok(keys)
    }

    /// Insert a new JWT signing key and deactivate all others (rotation).
    pub async fn insert_signing_key(
        &self,
        id: &str,
        algorithm: &str,
        public_key: &str,
        private_key: &str,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("beginning signing key rotation transaction")?;

        sqlx::query("UPDATE jwt_signing_keys SET active = false WHERE active = true")
            .execute(&mut *tx)
            .await
            .context("deactivating prior signing keys")?;

        sqlx::query(
            "INSERT INTO jwt_signing_keys (id, algorithm, public_key, private_key, active) \
             VALUES ($1, $2, $3, $4, true)",
        )
        .bind(id)
        .bind(algorithm)
        .bind(public_key)
        .bind(private_key)
        .execute(&mut *tx)
        .await
        .context("inserting new signing key")?;

        tx.commit()
            .await
            .context("committing signing key rotation")?;

        self.notify("signing_keys").await?;
        info!(key_id = id, algorithm, "JWT signing key activated");
        Ok(())
    }

    /// Deactivate a JWT signing key by ID.
    pub async fn deactivate_signing_key(&self, id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE jwt_signing_keys SET active = false WHERE id = $1 AND active = true",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .context("deactivating signing key")?;

        if result.rows_affected() > 0 {
            self.notify("signing_keys").await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ── Authorization policies ───────────────────────────────────────────────

    /// Load all ENABLED authorization policies, oldest first.
    ///
    /// Ordering is stable (`created_at`, then `id`) so the merged Cedar bundle
    /// is deterministic and the "first schema/entities wins" rule in
    /// [`crate::authz::CedarBundle::from_records`] is reproducible.
    pub async fn load_enabled_policies(&self) -> Result<Vec<PolicyRow>> {
        let rows = sqlx::query(
            "SELECT id, policy_text, schema_json, entities_json, enabled \
             FROM authz_policies WHERE enabled = true ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("loading enabled authz policies")?;

        let mut policies = Vec::with_capacity(rows.len());
        for r in rows {
            policies.push(PolicyRow::from_row(&r)?);
        }
        Ok(policies)
    }

    /// List all authorization policies (enabled and disabled), newest first.
    /// Includes `written_by` from the latest version row via LEFT JOIN.
    pub async fn list_policies(&self) -> Result<Vec<PolicyRow>> {
        let rows = sqlx::query(
            "SELECT p.id, p.policy_text, p.schema_json, p.entities_json, p.enabled, \
                    v.written_by \
             FROM authz_policies p \
             LEFT JOIN LATERAL ( \
               SELECT written_by FROM cedar_policy_versions \
               WHERE policy_id = p.id \
               ORDER BY version_num DESC LIMIT 1 \
             ) v ON true \
             ORDER BY p.created_at DESC, p.id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing authz policies")?;

        let mut policies = Vec::with_capacity(rows.len());
        for r in rows {
            policies.push(PolicyRow::from_row(&r)?);
        }
        Ok(policies)
    }

    /// Fetch a single authorization policy by id.
    /// Includes `written_by` from the latest version row via LEFT JOIN.
    pub async fn get_policy(&self, id: &str) -> Result<Option<PolicyRow>> {
        let row = sqlx::query(
            "SELECT p.id, p.policy_text, p.schema_json, p.entities_json, p.enabled, \
                    v.written_by \
             FROM authz_policies p \
             LEFT JOIN LATERAL ( \
               SELECT written_by FROM cedar_policy_versions \
               WHERE policy_id = p.id \
               ORDER BY version_num DESC LIMIT 1 \
             ) v ON true \
             WHERE p.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("fetching authz policy")?;

        match row {
            None => Ok(None),
            Some(r) => Ok(Some(PolicyRow::from_row(&r)?)),
        }
    }

    /// Insert or update an authorization policy (upsert on `id`).
    ///
    /// The caller MUST have validated `policy_text` (and `schema_json`) with the
    /// Cedar validator before calling this — the store is not a validation
    /// boundary. Emits a `policies` NOTIFY so peers reload.
    ///
    /// Every successful upsert atomically writes a `cedar_policy_versions` row so
    /// the full edit history is preserved. `written_by` is nullable; caller
    /// identity wiring is deferred — pass `None` until attribution is threaded
    /// through the admin handler stack.
    pub async fn upsert_policy(
        &self,
        id: &str,
        policy_text: &str,
        schema_json: Option<&serde_json::Value>,
        entities_json: Option<&serde_json::Value>,
        enabled: bool,
        written_by: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.context("beginning upsert_policy transaction")?;

        sqlx::query(
            "INSERT INTO authz_policies (id, policy_text, schema_json, entities_json, enabled, updated_at) \
             VALUES ($1, $2, $3, $4, $5, NOW()) \
             ON CONFLICT (id) DO UPDATE SET \
               policy_text = $2, schema_json = $3, entities_json = $4, enabled = $5, updated_at = NOW()",
        )
        .bind(id)
        .bind(policy_text)
        .bind(schema_json)
        .bind(entities_json)
        .bind(enabled)
        .execute(&mut *tx)
        .await
        .context("upserting authz policy")?;

        let next_version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version_num), 0) + 1 FROM cedar_policy_versions WHERE policy_id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .context("computing next policy version_num")?;

        sqlx::query(
            "INSERT INTO cedar_policy_versions \
             (policy_id, version_num, policy_text, schema_json, entities_json, written_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(next_version)
        .bind(policy_text)
        .bind(schema_json)
        .bind(entities_json)
        .bind(written_by)
        .execute(&mut *tx)
        .await
        .context("inserting policy version row")?;

        tx.commit().await.context("committing upsert_policy transaction")?;

        self.notify("policies").await?;
        info!(policy_id = id, enabled, version = next_version, "authz policy upserted");
        Ok(())
    }

    /// Delete an authorization policy by id. Returns `false` if not found.
    pub async fn delete_policy(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM authz_policies WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("deleting authz policy")?;

        if result.rows_affected() > 0 {
            self.notify("policies").await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ── Authorization audit trail ────────────────────────────────────────────

    /// Insert one authorization-decision audit row (parameterized).
    ///
    /// This is a fire-and-forget write on the decision path — callers spawn it on
    /// the Tokio runtime so a slow or failing insert never blocks or fails the
    /// request. The authorization decision itself is authoritative regardless of
    /// whether this row persisted, so an error here is logged and ignored by the
    /// caller. Mirrors [`Database::log_usage`].
    pub async fn log_authz_decision(&self, record: &AuthzAuditRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO authz_audit \
             (id, request_id, principal, action, resource, decision, reason, context) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::new_v4())
        .bind(&record.request_id)
        .bind(&record.principal)
        .bind(&record.action)
        .bind(&record.resource)
        .bind(record.decision.as_str())
        .bind(&record.reason)
        .bind(&record.context)
        .execute(&self.pool)
        .await
        .context("logging authz decision")?;

        debug!(
            principal = %record.principal,
            decision = %record.decision.as_str(),
            "authz decision logged"
        );
        Ok(())
    }

    /// List authorization-audit rows, newest first, with optional filters.
    ///
    /// All filters are parameterized. `principal` and `decision` are exact-match
    /// equality filters (a `None` disables that filter via the `$n IS NULL OR …`
    /// idiom so a single prepared statement covers every filter combination).
    /// `since`/`until` bound `created_at`. `limit`/`offset` page the result; the
    /// caller is expected to have already clamped `limit` to a sane cap.
    pub async fn list_authz_audit(&self, query: &AuditQuery) -> Result<Vec<AuditRow>> {
        let rows = sqlx::query(
            "SELECT id, request_id, principal, action, resource, decision, reason, context, created_at \
             FROM authz_audit \
             WHERE ($1::text IS NULL OR principal = $1) \
               AND ($2::text IS NULL OR decision = $2) \
               AND ($3::timestamptz IS NULL OR created_at >= $3) \
               AND ($4::timestamptz IS NULL OR created_at <= $4) \
             ORDER BY created_at DESC \
             LIMIT $5 OFFSET $6",
        )
        .bind(&query.principal)
        .bind(query.decision.as_ref().map(AuthzAuditDecision::as_str))
        .bind(query.since)
        .bind(query.until)
        .bind(query.limit)
        .bind(query.offset)
        .fetch_all(&self.pool)
        .await
        .context("listing authz audit rows")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(AuditRow::from_row(&r)?);
        }
        Ok(out)
    }

    // ── Policy version history ────────────────────────────────────────────────

    /// List version rows for a policy, newest-first, with pagination.
    ///
    /// Returns an empty `Vec` when the policy has no version history yet (e.g.
    /// it was created before versioning was introduced). `limit` is expected to
    /// be pre-clamped by the caller (admin handler clamps to 100).
    pub async fn list_policy_versions(
        &self,
        policy_id: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<PolicyVersionRow>> {
        let rows = sqlx::query(
            "SELECT id, policy_id, version_num, policy_text, schema_json, entities_json, written_by, written_at \
             FROM cedar_policy_versions \
             WHERE policy_id = $1 \
             ORDER BY version_num DESC \
             LIMIT $2 OFFSET $3",
        )
        .bind(policy_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .context("listing policy versions")?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(PolicyVersionRow::from_row(&r)?);
        }
        Ok(out)
    }

    /// Fetch a single version row by `(policy_id, version_num)`.
    ///
    /// Returns `None` when the version does not exist (caller returns 404).
    pub async fn get_policy_version(
        &self,
        policy_id: &str,
        version_num: i32,
    ) -> Result<Option<PolicyVersionRow>> {
        let row = sqlx::query(
            "SELECT id, policy_id, version_num, policy_text, schema_json, entities_json, written_by, written_at \
             FROM cedar_policy_versions \
             WHERE policy_id = $1 AND version_num = $2",
        )
        .bind(policy_id)
        .bind(version_num)
        .fetch_optional(&self.pool)
        .await
        .context("fetching policy version")?;

        match row {
            None => Ok(None),
            Some(r) => Ok(Some(PolicyVersionRow::from_row(&r)?)),
        }
    }
}

/// A route row from the `gate_routes` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRoute {
    pub id: String,
    pub config: serde_json::Value,
    pub priority: i32,
    pub enabled: bool,
}

/// A version row from the `cedar_policy_versions` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersionRow {
    pub id: i32,
    pub policy_id: String,
    pub version_num: i32,
    pub policy_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_json: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities_json: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_by: Option<String>,
    pub written_at: DateTime<Utc>,
}

impl PolicyVersionRow {
    fn from_row(r: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            id: r.try_get("id")?,
            policy_id: r.try_get("policy_id")?,
            version_num: r.try_get("version_num")?,
            policy_text: r.try_get("policy_text")?,
            schema_json: r.try_get("schema_json")?,
            entities_json: r.try_get("entities_json")?,
            written_by: r.try_get("written_by")?,
            written_at: r.try_get("written_at")?,
        })
    }
}

/// An authorization policy row from the `authz_policies` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRow {
    pub id: String,
    pub policy_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_json: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities_json: Option<serde_json::Value>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_by: Option<String>,
}

impl PolicyRow {
    /// Build a `PolicyRow` from a sqlx row selecting the standard columns.
    fn from_row(r: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            id: r.try_get("id")?,
            policy_text: r.try_get("policy_text")?,
            schema_json: r.try_get("schema_json")?,
            entities_json: r.try_get("entities_json")?,
            enabled: r.try_get("enabled")?,
            written_by: r.try_get("written_by").unwrap_or(None),
        })
    }

    /// Convert into the authz engine's [`crate::authz::PolicyRecord`].
    pub fn into_record(self) -> crate::authz::PolicyRecord {
        crate::authz::PolicyRecord {
            id: self.id,
            policy_text: self.policy_text,
            schema_json: self.schema_json,
            entities_json: self.entities_json,
        }
    }
}

/// The authorization decision recorded in the `authz_audit.decision` column.
///
/// Serializes to a stable lowercase string. `Approval` is reserved for the
/// later HITL-approval change and is a valid value now so the schema/enum need
/// not change when that decision point is wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthzAuditDecision {
    Allow,
    Deny,
    StepUp,
    Approval,
}

impl AuthzAuditDecision {
    /// The canonical string persisted in and matched against the `decision`
    /// TEXT column. Kept explicit (not derived from `Debug`) so the wire value
    /// is stable independent of the Rust identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthzAuditDecision::Allow => "allow",
            AuthzAuditDecision::Deny => "deny",
            AuthzAuditDecision::StepUp => "step_up",
            AuthzAuditDecision::Approval => "approval",
        }
    }

    /// Parse a `decision` filter value (as accepted by the admin `/audit`
    /// endpoint) into a decision. Returns `None` for an unrecognized value so
    /// the caller can reject it rather than silently match nothing.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(AuthzAuditDecision::Allow),
            "deny" => Some(AuthzAuditDecision::Deny),
            "step_up" => Some(AuthzAuditDecision::StepUp),
            "approval" => Some(AuthzAuditDecision::Approval),
            _ => None,
        }
    }
}

impl std::fmt::Display for AuthzAuditDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An authorization-decision audit record to be logged via
/// [`Database::log_authz_decision`]. Constructed on the decision path and
/// written best-effort/non-blocking (the `id` and `created_at` are assigned by
/// the insert, so they are not carried here).
#[derive(Debug, Clone)]
pub struct AuthzAuditRecord {
    pub request_id: Option<String>,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub decision: AuthzAuditDecision,
    pub reason: Option<String>,
    pub context: Option<serde_json::Value>,
}

/// Filters + paging for [`Database::list_authz_audit`]. `limit`/`offset` are
/// `i64` to bind directly into Postgres `LIMIT`/`OFFSET`; the admin handler
/// clamps them before constructing this.
#[derive(Debug, Clone)]
pub struct AuditQuery {
    pub principal: Option<String>,
    pub decision: Option<AuthzAuditDecision>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

/// An authorization-audit row read back from `authz_audit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

impl AuditRow {
    /// Build an `AuditRow` from a sqlx row selecting the standard columns.
    fn from_row(r: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            id: r.try_get("id")?,
            request_id: r.try_get("request_id")?,
            principal: r.try_get("principal")?,
            action: r.try_get("action")?,
            resource: r.try_get("resource")?,
            decision: r.try_get("decision")?,
            reason: r.try_get("reason")?,
            context: r.try_get("context")?,
            created_at: r.try_get("created_at")?,
        })
    }
}

/// An API key record from the `api_keys` table.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub client_id: String,
    pub role: String,
    pub principal_type: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// An OAuth client record from the `oauth_clients` table (no secret hash).
#[derive(Debug, Clone)]
pub struct OAuthClientRecord {
    pub id: Uuid,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub audience: Option<String>,
}

/// A non-human-identity record from the `agent_identities` table.
#[derive(Debug, Clone, Serialize)]
pub struct AgentIdentityRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub label: Option<String>,
    pub rotated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A usage event to be logged via [`Database::log_usage`].
#[derive(Debug)]
pub struct UsageEvent {
    pub id: Uuid,
    pub request_id: String,
    pub user_id: String,
    pub route_id: String,
    pub tokens: i64,
    pub duration_ms: i64,
    pub metadata: serde_json::Value,
}

/// A JWT signing key row from the database (includes private key — internal only).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct JwtSigningKey {
    pub id: String,
    pub algorithm: String,
    pub public_key: String,
    pub private_key: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// Public projection of a JWT signing key (no private key — safe for API responses).
#[derive(Debug, Clone, Serialize)]
pub struct JwtSigningKeyPublic {
    pub id: String,
    pub algorithm: String,
    pub public_key: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

impl UsageEvent {
    pub fn new(
        request_id: impl Into<String>,
        user_id: impl Into<String>,
        route_id: impl Into<String>,
        tokens: u64,
        duration_ms: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            request_id: request_id.into(),
            user_id: user_id.into(),
            route_id: route_id.into(),
            tokens: tokens as i64,
            duration_ms: duration_ms as i64,
            metadata: serde_json::Value::Object(Default::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AuthzAuditDecision;
    use crate::config::types::BudgetWindow;

    /// The `decision` column value is stable and lowercase (with `step_up`
    /// snake-cased). A change to any of these wire strings breaks stored-row
    /// comparability and the admin `/audit` filter, so pin them explicitly.
    #[test]
    fn authz_audit_decision_serializes_to_stable_strings() {
        assert_eq!(AuthzAuditDecision::Allow.as_str(), "allow");
        assert_eq!(AuthzAuditDecision::Deny.as_str(), "deny");
        assert_eq!(AuthzAuditDecision::StepUp.as_str(), "step_up");
        assert_eq!(AuthzAuditDecision::Approval.as_str(), "approval");
    }

    /// `parse` round-trips every known decision and rejects anything else so an
    /// unknown `?decision=` filter is a 400, not a silent empty match.
    #[test]
    fn authz_audit_decision_parse_round_trips_and_rejects_unknown() {
        for d in [
            AuthzAuditDecision::Allow,
            AuthzAuditDecision::Deny,
            AuthzAuditDecision::StepUp,
            AuthzAuditDecision::Approval,
        ] {
            assert_eq!(AuthzAuditDecision::parse(d.as_str()), Some(d));
        }
        assert_eq!(AuthzAuditDecision::parse("nope"), None);
        assert_eq!(AuthzAuditDecision::parse("Allow"), None);
        assert_eq!(AuthzAuditDecision::parse("stepup"), None);
    }

    /// The windowed fallback binds `window.pg_interval()` into the query's
    /// `$2::interval` placeholder. This asserts the exact literals so a change
    /// to the window→interval contract is caught without a live database.
    #[test]
    fn windowed_fallback_binds_expected_pg_intervals() {
        assert_eq!(BudgetWindow::Minute.pg_interval(), Some("1 minute"));
        assert_eq!(BudgetWindow::Hour.pg_interval(), Some("1 hour"));
        assert_eq!(BudgetWindow::Day.pg_interval(), Some("1 day"));
        // Lifetime has no time bound and never reaches the windowed query.
        assert_eq!(BudgetWindow::Lifetime.pg_interval(), None);
    }

    /// Integration test for `get_user_token_total_windowed`. Requires a live
    /// Postgres at `DATABASE_URL` and is `#[ignore]`d so the default test run
    /// needs no database. Run explicitly with:
    ///   DATABASE_URL=postgres://... cargo test -p flint-gate-core --all-features -- --ignored
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn windowed_total_sums_only_recent_events() {
        use super::{Database, UsageEvent};
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let user = format!("wtest-{}", uuid::Uuid::new_v4());
        db.log_usage(&UsageEvent::new("r1", &user, "route", 300, 5))
            .await
            .unwrap();

        // A generous window includes the just-written event.
        let hour = db
            .get_user_token_total_windowed(&user, "1 hour")
            .await
            .unwrap();
        assert_eq!(hour, 300);

        // A negative-ish window would exclude it; use a tiny interval to prove
        // the time bound is applied (the row is newer than now()-'0 seconds' is
        // false, so it must be excluded).
        let none = db
            .get_user_token_total_windowed(&user, "0 seconds")
            .await
            .unwrap();
        assert_eq!(none, 0);
    }

    /// Round-trip test for the authz audit trail. Requires a live Postgres at
    /// `DATABASE_URL` and is `#[ignore]`d so the default test run needs no
    /// database. Run explicitly with:
    ///   DATABASE_URL=postgres://... cargo test -p flint-gate-core --all-features -- --ignored
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn authz_audit_write_and_filtered_read_round_trip() {
        use super::{AuditQuery, AuthzAuditDecision, AuthzAuditRecord, Database};
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let principal = format!("audit-{}", uuid::Uuid::new_v4());
        db.log_authz_decision(&AuthzAuditRecord {
            request_id: Some("req-1".to_string()),
            principal: principal.clone(),
            action: "invoke".to_string(),
            resource: "route-x".to_string(),
            decision: AuthzAuditDecision::Deny,
            reason: Some("policy denied".to_string()),
            context: Some(serde_json::json!({"method": "GET"})),
        })
        .await
        .unwrap();

        // Filter by principal + decision returns exactly the written row.
        let rows = db
            .list_authz_audit(&AuditQuery {
                principal: Some(principal.clone()),
                decision: Some(AuthzAuditDecision::Deny),
                since: None,
                until: None,
                limit: 100,
                offset: 0,
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].principal, principal);
        assert_eq!(rows[0].decision, "deny");

        // A non-matching decision filter yields nothing.
        let allow_rows = db
            .list_authz_audit(&AuditQuery {
                principal: Some(principal),
                decision: Some(AuthzAuditDecision::Allow),
                since: None,
                until: None,
                limit: 100,
                offset: 0,
            })
            .await
            .unwrap();
        assert!(allow_rows.is_empty());
    }

    #[test]
    fn sha256_hex_is_stable_and_hex() {
        let h = super::sha256_hex("secret");
        // SHA-256 → 32 bytes → 64 hex chars, deterministic.
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(h, super::sha256_hex("secret"));
        assert_ne!(h, super::sha256_hex("secre7"));
    }

    // ── SecretHash (bcrypt + legacy SHA-256 format-sniff) ─────────────────

    #[test]
    fn secret_hash_bcrypt_round_trip() {
        let hash = super::SecretHash::hash("s3cr3t-token").expect("bcrypt hash");
        assert!(super::SecretHash::is_bcrypt(&hash));
        assert!(super::SecretHash::verify("s3cr3t-token", &hash));
        assert!(!super::SecretHash::verify("wrong", &hash));
        // A fresh bcrypt hash does not need a rehash.
        assert!(!super::SecretHash::needs_rehash(&hash));
        // Each hash is salted → two hashes of the same secret differ.
        let hash2 = super::SecretHash::hash("s3cr3t-token").unwrap();
        assert_ne!(hash, hash2);
    }

    #[test]
    fn secret_hash_verifies_legacy_sha256_and_flags_rehash() {
        // A legacy row stores the raw sha256_hex; it must still verify, and be
        // flagged for upgrade to bcrypt.
        let legacy = super::sha256_hex("legacy-secret");
        assert!(!super::SecretHash::is_bcrypt(&legacy));
        assert!(super::SecretHash::verify("legacy-secret", &legacy));
        assert!(!super::SecretHash::verify("nope", &legacy));
        assert!(super::SecretHash::needs_rehash(&legacy));
    }

    #[test]
    fn secret_hash_rejects_over_length_secret() {
        // >72 bytes would be silently truncated by bcrypt — reject instead.
        let over = "a".repeat(73);
        assert!(super::SecretHash::hash(&over).is_err());
        // 72 bytes is the boundary and is accepted.
        assert!(super::SecretHash::hash(&"a".repeat(72)).is_ok());
    }

    #[test]
    fn secret_hash_verify_never_panics_on_garbage() {
        // An unparseable / truncated hash → false, never a panic.
        assert!(!super::SecretHash::verify("x", "$2b$not-a-real-bcrypt-hash"));
        assert!(!super::SecretHash::verify("x", ""));
        assert!(!super::SecretHash::verify("x", "short"));
    }

    /// Round-trip for the OAuth client store: create → verify good secret →
    /// verify bad secret denied. Requires a live Postgres via `DATABASE_URL`.
    ///   DATABASE_URL=postgres://... cargo test -p flint-gate-core --all-features -- --ignored
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn oauth_client_create_and_verify_round_trip() {
        use super::Database;
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let client_id = format!("svc-{}", uuid::Uuid::new_v4());
        let (_, raw_secret) = db
            .create_oauth_client(&client_id, &["svc.read".into()], Some("api"))
            .await
            .unwrap();

        // Correct secret verifies and returns the grant.
        let ok = db
            .verify_client_credentials(&client_id, &raw_secret)
            .await
            .unwrap();
        let rec = ok.expect("valid credentials verify");
        assert_eq!(rec.client_id, client_id);
        assert_eq!(rec.scopes, vec!["svc.read".to_string()]);
        assert_eq!(rec.audience.as_deref(), Some("api"));

        // Wrong secret is denied (no fail-open).
        let bad = db
            .verify_client_credentials(&client_id, "wrong-secret")
            .await
            .unwrap();
        assert!(bad.is_none());

        // Unknown client is denied.
        let unknown = db
            .verify_client_credentials("no-such-client", &raw_secret)
            .await
            .unwrap();
        assert!(unknown.is_none());

        // The stored hash for a freshly-created client is bcrypt.
        let stored: String =
            sqlx::query_scalar("SELECT secret_hash FROM oauth_clients WHERE client_id = $1")
                .bind(&client_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(super::SecretHash::is_bcrypt(&stored));

        // ── Legacy SHA-256 migration ──────────────────────────────────────
        // Insert a client with a raw legacy hash, verify it still authenticates,
        // and confirm the stored hash is transparently upgraded to bcrypt.
        let legacy_id = format!("legacy-{}", uuid::Uuid::new_v4());
        let legacy_secret = "legacy-plaintext-secret";
        sqlx::query(
            "INSERT INTO oauth_clients (client_id, secret_hash, scopes) VALUES ($1, $2, '[]')",
        )
        .bind(&legacy_id)
        .bind(super::sha256_hex(legacy_secret))
        .execute(&db.pool)
        .await
        .unwrap();

        assert!(db
            .verify_client_credentials(&legacy_id, legacy_secret)
            .await
            .unwrap()
            .is_some());

        let upgraded: String =
            sqlx::query_scalar("SELECT secret_hash FROM oauth_clients WHERE client_id = $1")
                .bind(&legacy_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(
            super::SecretHash::is_bcrypt(&upgraded),
            "legacy hash should be upgraded to bcrypt on verify"
        );
    }

    /// Round-trip for the NHI lifecycle: issue → not-revoked → revoke →
    /// revoked. Requires a live Postgres via `DATABASE_URL`.
    ///   DATABASE_URL=postgres://... cargo test -p flint-gate-core --all-features -- --ignored
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn agent_identity_lifecycle_round_trip() {
        use super::Database;
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let id = format!("agent-{}", uuid::Uuid::new_v4());

        // An id that was never issued is not on the revocation list.
        assert!(!db.is_agent_revoked(&id).await.unwrap());

        db.issue_agent_identity(&id, "agent", Some("test bot"))
            .await
            .unwrap();
        assert!(!db.is_agent_revoked(&id).await.unwrap());

        // Rotate stamps rotated_at, stays active.
        assert!(db.rotate_agent_identity(&id).await.unwrap());
        assert!(!db.is_agent_revoked(&id).await.unwrap());

        // Revoke → is_revoked flips true (denied on next authorize).
        assert!(db.revoke_agent_identity(&id).await.unwrap());
        assert!(db.is_agent_revoked(&id).await.unwrap());

        // Revoking again is a no-op (already revoked).
        assert!(!db.revoke_agent_identity(&id).await.unwrap());

        // It appears in the listing.
        let list = db.list_agent_identities().await.unwrap();
        assert!(list.iter().any(|r| r.id == id && r.status == "revoked"));

        // Each lifecycle event was audited transactionally (audited-before-effect):
        // issue + rotate + revoke → exactly 3 audit rows for this principal, all
        // with resource='agent_identity'. Revoking-again was a no-op → no 4th row.
        let audit_actions: Vec<String> = sqlx::query_scalar(
            "SELECT action FROM authz_audit WHERE principal = $1 AND resource = 'agent_identity' ORDER BY created_at",
        )
        .bind(&id)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            audit_actions,
            vec![
                "nhi_issue".to_string(),
                "nhi_rotate".to_string(),
                "nhi_revoke".to_string()
            ]
        );
    }

    /// Policy version history: first write → version_num 1; second write →
    /// version_num 2; `get_policy_version` returns the correct row.
    /// Requires a live Postgres at `DATABASE_URL`.
    ///   DATABASE_URL=postgres://... cargo test -p flint-gate-core --all-features -- --ignored
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn policy_version_history_increments_and_fetches() {
        use super::Database;
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let policy_id = format!("ver-test-{}", uuid::Uuid::new_v4());
        let text_v1 = "permit(principal, action, resource);";
        let text_v2 = "forbid(principal, action, resource);";

        // First upsert → version_num = 1
        db.upsert_policy(&policy_id, text_v1, None, None, true, None)
            .await
            .unwrap();

        let versions = db.list_policy_versions(&policy_id, 0, 10).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version_num, 1);
        assert_eq!(versions[0].policy_text, text_v1);
        assert!(versions[0].written_by.is_none());

        // Second upsert → version_num = 2
        db.upsert_policy(&policy_id, text_v2, None, None, true, Some("test-user"))
            .await
            .unwrap();

        let versions2 = db.list_policy_versions(&policy_id, 0, 10).await.unwrap();
        assert_eq!(versions2.len(), 2);
        // Newest first
        assert_eq!(versions2[0].version_num, 2);
        assert_eq!(versions2[0].policy_text, text_v2);
        assert_eq!(versions2[0].written_by.as_deref(), Some("test-user"));
        assert_eq!(versions2[1].version_num, 1);

        // get_policy_version returns the correct row
        let v1 = db.get_policy_version(&policy_id, 1).await.unwrap().unwrap();
        assert_eq!(v1.policy_text, text_v1);
        assert_eq!(v1.version_num, 1);

        let v2 = db.get_policy_version(&policy_id, 2).await.unwrap().unwrap();
        assert_eq!(v2.policy_text, text_v2);
        assert_eq!(v2.written_by.as_deref(), Some("test-user"));

        // Unknown version returns None
        assert!(db.get_policy_version(&policy_id, 99).await.unwrap().is_none());
    }

    /// ON DELETE CASCADE: deleting a policy removes all its version rows.
    /// Requires a live Postgres at `DATABASE_URL`.
    #[tokio::test]
    #[ignore = "requires a live Postgres via DATABASE_URL"]
    async fn policy_version_cascade_delete() {
        use super::Database;
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
        let db = Database::connect(&url, 2).await.unwrap();
        db.migrate().await.unwrap();

        let policy_id = format!("cascade-test-{}", uuid::Uuid::new_v4());
        db.upsert_policy(&policy_id, "permit(principal, action, resource);", None, None, true, None)
            .await
            .unwrap();
        db.upsert_policy(&policy_id, "forbid(principal, action, resource);", None, None, true, None)
            .await
            .unwrap();

        // Two version rows exist before delete
        let before = db.list_policy_versions(&policy_id, 0, 10).await.unwrap();
        assert_eq!(before.len(), 2);

        // Delete the policy — cascade should remove version rows
        assert!(db.delete_policy(&policy_id).await.unwrap());

        // Version rows are gone (ON DELETE CASCADE)
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cedar_policy_versions WHERE policy_id = $1",
        )
        .bind(&policy_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "cascade delete must remove all version rows");
    }
}
