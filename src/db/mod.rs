/// Database access layer — wraps `sqlx::PgPool` with Flint Gate schema operations.
///
/// Schema is applied via `migrate()` at startup. All mutations emit a Postgres
/// NOTIFY on `flintgate_config_changed` so that other instances invalidate caches.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
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
            routes.push(DbRoute { id, config, priority, enabled });
        }
        Ok(routes)
    }

    /// Upsert a route configuration.
    pub async fn upsert_route(&self, id: &str, config: &serde_json::Value, priority: i32) -> Result<()> {
        sqlx::query(
            "INSERT INTO gate_routes (id, config, priority, updated_at) VALUES ($1, $2, $3, NOW())
             ON CONFLICT (id) DO UPDATE SET config = $2, priority = $3, updated_at = NOW()"
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
             WHERE key_hash = $1 AND active = true AND (expires_at IS NULL OR expires_at > NOW())"
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

                Ok(Some(ApiKeyRecord { id, client_id, scopes, expires_at }))
            }
        }
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
