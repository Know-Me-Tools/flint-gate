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

/// DDL for the Flint Gate schema. Applied at startup via `migrate()`.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS gate_routes (
    id          TEXT PRIMARY KEY,
    config      JSONB NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 0,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS gate_sites (
    id              TEXT PRIMARY KEY,
    domains         JSONB NOT NULL DEFAULT '[]',
    default_auth    TEXT,
    default_upstream TEXT,
    config          JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS api_keys (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash    TEXT NOT NULL UNIQUE,
    client_id   TEXT NOT NULL,
    scopes      JSONB NOT NULL DEFAULT '[]',
    active      BOOLEAN NOT NULL DEFAULT true,
    expires_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS usage_events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id  TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    route_id    TEXT NOT NULL,
    tokens      BIGINT NOT NULL DEFAULT 0,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    metadata    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS jwt_signing_keys (
    id          TEXT PRIMARY KEY,
    algorithm   TEXT NOT NULL,
    public_key  TEXT NOT NULL,
    private_key TEXT NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS authz_policies (
    id            TEXT PRIMARY KEY,
    policy_text   TEXT NOT NULL,
    schema_json   JSONB,
    entities_json JSONB,
    enabled       BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS authz_audit (
    id          UUID PRIMARY KEY,
    request_id  TEXT,
    principal   TEXT NOT NULL,
    action      TEXT NOT NULL,
    resource    TEXT NOT NULL,
    decision    TEXT NOT NULL,
    reason      TEXT,
    context     JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS authz_audit_created_at_idx ON authz_audit (created_at DESC);
CREATE INDEX IF NOT EXISTS authz_audit_principal_idx ON authz_audit (principal);
"#;

/// Database access wrapper.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
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

    /// Apply the DDL schema (idempotent — uses `CREATE TABLE IF NOT EXISTS`).
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(SCHEMA_SQL)
            .execute(&self.pool)
            .await
            .context("applying schema")?;
        info!("database schema applied");
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
            "SELECT id, client_id, scopes, expires_at FROM api_keys
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
        use sha2::Digest;

        // Generate a 32-byte random key and encode as hex (64-char string).
        let raw_bytes: [u8; 32] = rand::thread_rng().gen();
        let raw_key = hex::encode(raw_bytes);
        let key_hash = {
            let mut h = sha2::Sha256::new();
            h.update(raw_key.as_bytes());
            hex::encode(h.finalize())
        };

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
            "SELECT id, client_id, scopes, expires_at FROM api_keys WHERE active = true ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing API keys")?;

        let mut keys = Vec::with_capacity(rows.len());
        for r in rows {
            let id: Uuid = r.try_get("id")?;
            let client_id: String = r.try_get("client_id")?;
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
    pub async fn list_policies(&self) -> Result<Vec<PolicyRow>> {
        let rows = sqlx::query(
            "SELECT id, policy_text, schema_json, entities_json, enabled \
             FROM authz_policies ORDER BY created_at DESC, id ASC",
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
    pub async fn get_policy(&self, id: &str) -> Result<Option<PolicyRow>> {
        let row = sqlx::query(
            "SELECT id, policy_text, schema_json, entities_json, enabled \
             FROM authz_policies WHERE id = $1",
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
    pub async fn upsert_policy(
        &self,
        id: &str,
        policy_text: &str,
        schema_json: Option<&serde_json::Value>,
        entities_json: Option<&serde_json::Value>,
        enabled: bool,
    ) -> Result<()> {
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
        .execute(&self.pool)
        .await
        .context("upserting authz policy")?;

        self.notify("policies").await?;
        info!(policy_id = id, enabled, "authz policy upserted");
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
}

/// A route row from the `gate_routes` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRoute {
    pub id: String,
    pub config: serde_json::Value,
    pub priority: i32,
    pub enabled: bool,
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
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
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
}
