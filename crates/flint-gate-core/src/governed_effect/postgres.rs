use super::{
    AuthenticatedAdmin, ChallengeDecision, ChallengeRecord, ChallengeStore,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub struct PostgresChallengeStore {
    pool: PgPool,
}

impl PostgresChallengeStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChallengeStore for PostgresChallengeStore {
    async fn register(&self, challenge: ChallengeRecord) -> Result<()> {
        let request = serde_json::to_value(&challenge.request)?;
        let binding = serde_json::to_value(&challenge.binding)?;
        let result = sqlx::query(
            "INSERT INTO governed_effect_approvals
             (issuer, challenge_id, effect_id, invocation_id, request_json,
              request_sha256, authority_binding, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (issuer, challenge_id) DO NOTHING",
        )
        .bind(&challenge.issuer)
        .bind(challenge.challenge_id)
        .bind(challenge.request.effect_id)
        .bind(challenge.request.invocation_id)
        .bind(request)
        .bind(&challenge.binding.request_sha256)
        .bind(binding)
        .bind(challenge.created_at)
        .bind(challenge.expires_at)
        .execute(&self.pool)
        .await
        .context("registering governed-effect approval")?;
        if result.rows_affected() == 0 {
            let existing = self
                .load(&challenge.issuer, challenge.challenge_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("challenge conflict disappeared"))?;
            if existing.binding != challenge.binding {
                anyhow::bail!("governed-effect challenge identity collision");
            }
        }
        Ok(())
    }

    async fn load(&self, issuer: &str, challenge_id: Uuid) -> Result<Option<ChallengeRecord>> {
        let row = sqlx::query(
            "SELECT issuer, challenge_id, request_json, authority_binding,
                    created_at, expires_at, decision, decided_at,
                    decided_by_issuer, decided_by_subject
             FROM governed_effect_approvals
             WHERE issuer = $1 AND challenge_id = $2",
        )
        .bind(issuer)
        .bind(challenge_id)
        .fetch_optional(&self.pool)
        .await
        .context("loading governed-effect approval")?;

        row.map(|row| {
            let request = serde_json::from_value(
                row.try_get("request_json").context("reading request_json")?,
            )?;
            let binding = serde_json::from_value(
                row.try_get("authority_binding")
                    .context("reading authority_binding")?,
            )?;
            let decision = match row
                .try_get::<Option<String>, _>("decision")
                .context("reading decision")?
                .as_deref()
            {
                Some("approve") => Some(ChallengeDecision::Approve),
                Some("deny") => Some(ChallengeDecision::Deny),
                _ => None,
            };
            let decided_by_subject: Option<String> = row
                .try_get("decided_by_subject")
                .context("reading decided_by_subject")?;
            let decided_by_issuer: Option<String> = row
                .try_get("decided_by_issuer")
                .context("reading decided_by_issuer")?;
            let decided_by = decided_by_subject.zip(decided_by_issuer).map(
                |(subject, issuer)| AuthenticatedAdmin { issuer, subject },
            );
            Ok(ChallengeRecord {
                issuer: row.try_get("issuer").context("reading issuer")?,
                challenge_id: row
                    .try_get("challenge_id")
                    .context("reading challenge_id")?,
                request,
                binding,
                created_at: row.try_get("created_at").context("reading created_at")?,
                expires_at: row.try_get("expires_at").context("reading expires_at")?,
                decision,
                decided_at: row.try_get("decided_at").context("reading decided_at")?,
                decided_by,
            })
        })
        .transpose()
    }

    async fn decide(
        &self,
        issuer: &str,
        challenge_id: Uuid,
        decision: ChallengeDecision,
        admin: &AuthenticatedAdmin,
    ) -> Result<bool> {
        let decision = match decision {
            ChallengeDecision::Approve => "approve",
            ChallengeDecision::Deny => "deny",
        };
        let result = sqlx::query(
            "UPDATE governed_effect_approvals
             SET decision = $1, decided_at = NOW(), decided_by_issuer = $2,
                 decided_by_subject = $3
             WHERE issuer = $4 AND challenge_id = $5 AND decision IS NULL
               AND expires_at > NOW()",
        )
        .bind(decision)
        .bind(&admin.issuer)
        .bind(&admin.subject)
        .bind(issuer)
        .bind(challenge_id)
        .execute(&self.pool)
        .await
        .context("deciding governed-effect approval")?;
        Ok(result.rows_affected() == 1)
    }
}
