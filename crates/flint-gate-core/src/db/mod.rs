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
}

/// A route row from the `gate_routes` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRoute {
    pub id: String,
    pub config: serde_json::Value,
    pub priority: i32,
    pub enabled: bool,
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
