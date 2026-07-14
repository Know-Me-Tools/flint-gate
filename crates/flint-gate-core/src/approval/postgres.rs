/// Postgres-backed `ApprovalStore` implementation.
///
/// Uses the `pending_approvals` table (see migration
/// `0003_pending_approvals.sql`). Emits `pg_notify` on decide() so that other
/// replicas waiting on the same approval ID can wake up and re-check.
use super::{ApprovalDecision, ApprovalStatus, ApprovalStore, PendingApproval};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::{debug, warn};
use uuid::Uuid;

/// Postgres NOTIFY channel used to wake replicas when a decision is recorded.
const APPROVAL_CHANNEL: &str = "flintgate_approval_decided";

/// Postgres-backed approval store — durable, cross-replica.
pub struct PostgresApprovalStore {
    pool: PgPool,
}

impl PostgresApprovalStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ApprovalStore for PostgresApprovalStore {
    async fn register(
        &self,
        agent_sub: &str,
        tool_name: &str,
        reason: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        let row = sqlx::query(
            "INSERT INTO pending_approvals \
             (agent_sub, tool_name, reason, expires_at) \
             VALUES ($1, $2, $3, $4) \
             RETURNING id",
        )
        .bind(agent_sub)
        .bind(tool_name)
        .bind(reason)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .context("inserting pending approval")?;

        let id: Uuid = row
            .try_get("id")
            .context("reading id from pending_approvals insert")?;
        debug!(%id, agent_sub, tool_name, "approval registered");
        Ok(id)
    }

    async fn decide(&self, id: Uuid, decision: ApprovalDecision) -> Result<bool> {
        let decision_str = match decision {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Rejected => "rejected",
        };

        let result = sqlx::query(
            "UPDATE pending_approvals \
             SET decision = $1, decided_at = NOW() \
             WHERE id = $2 AND decision IS NULL",
        )
        .bind(decision_str)
        .bind(id)
        .execute(&self.pool)
        .await
        .context("recording approval decision")?;

        if result.rows_affected() == 0 {
            return Ok(false);
        }

        // Wake any replica waiting on this approval.
        if let Err(e) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(APPROVAL_CHANNEL)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
        {
            warn!(%id, error = %e, "pg_notify for approval decision failed");
        }

        debug!(%id, decision = decision_str, "approval decided");
        Ok(true)
    }

    async fn list(&self) -> Result<Vec<PendingApproval>> {
        let rows = sqlx::query(
            "SELECT id, agent_sub, tool_name, reason, \
                    registered_at, expires_at, decision, decided_at \
             FROM pending_approvals \
             WHERE decision IS NULL AND expires_at > NOW() \
             ORDER BY registered_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .context("listing pending approvals")?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row_to_approval(&row)?);
        }
        Ok(out)
    }

    async fn status(&self, id: Uuid) -> Result<Option<ApprovalStatus>> {
        let row = sqlx::query(
            "SELECT id, decision, decided_at, expires_at \
             FROM pending_approvals WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("querying approval status")?;

        match row {
            None => Ok(None),
            Some(r) => {
                let decision_str: Option<String> =
                    r.try_get("decision").context("reading decision column")?;
                Ok(Some(ApprovalStatus {
                    id: r.try_get("id").context("reading id")?,
                    decision: parse_decision(decision_str.as_deref()),
                    decided_at: r.try_get("decided_at").context("reading decided_at")?,
                    expires_at: r.try_get("expires_at").context("reading expires_at")?,
                }))
            }
        }
    }

    async fn purge_expired(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM pending_approvals WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .context("purging expired approvals")?;
        let n = result.rows_affected();
        if n > 0 {
            debug!(deleted = n, "purged expired approvals");
        }
        Ok(n)
    }

    async fn earliest_expiry(&self) -> Result<Option<DateTime<Utc>>> {
        let row = sqlx::query(
            "SELECT MIN(expires_at) AS earliest \
             FROM pending_approvals \
             WHERE decision IS NULL AND expires_at > NOW()",
        )
        .fetch_one(&self.pool)
        .await
        .context("querying earliest expiry")?;

        let earliest: Option<DateTime<Utc>> =
            row.try_get("earliest").context("reading earliest column")?;
        Ok(earliest)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_approval(row: &sqlx::postgres::PgRow) -> Result<PendingApproval> {
    let decision_str: Option<String> = row.try_get("decision").context("decision")?;
    Ok(PendingApproval {
        id: row.try_get("id").context("id")?,
        agent_sub: row.try_get("agent_sub").context("agent_sub")?,
        tool_name: row.try_get("tool_name").context("tool_name")?,
        reason: row.try_get("reason").context("reason")?,
        registered_at: row.try_get("registered_at").context("registered_at")?,
        expires_at: row.try_get("expires_at").context("expires_at")?,
        decision: parse_decision(decision_str.as_deref()),
        decided_at: row.try_get("decided_at").context("decided_at")?,
    })
}

fn parse_decision(s: Option<&str>) -> Option<ApprovalDecision> {
    match s {
        Some("approved") => Some(ApprovalDecision::Approved),
        Some("rejected") => Some(ApprovalDecision::Rejected),
        _ => None,
    }
}
