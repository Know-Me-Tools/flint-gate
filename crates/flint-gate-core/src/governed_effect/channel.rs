//! Gate-owned, persisted authority for channel effects. A permit authorizes
//! one exact boundary crossing; it does not execute or schedule that effect.

use super::{AuthenticatedAdmin, ExecutionOwnerAttestation, PayloadDigest, VerifiedIdentity};
use crate::authz::{AuthzDecision, AuthzEngine};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::sync::Arc;
use uuid::Uuid;

pub const CHANNEL_PROTOCOL: &str = "afc.channel-effect/1";
pub const CHANNEL_AUTHORITY_CONTRACT: &str = "afc.channel-authority/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAction {
    SourceDisclosure,
    RecipientDelivery,
    HandlerExecution,
    ScopedReply,
    RouteReassignment,
}

impl ChannelAction {
    fn cedar_action(self) -> &'static str {
        match self {
            Self::SourceDisclosure => "afc.channel:source-disclosure/1",
            Self::RecipientDelivery => "afc.channel:recipient-delivery/1",
            Self::HandlerExecution => "afc.channel:handler-execution/1",
            Self::ScopedReply => "afc.channel:scoped-reply/1",
            Self::RouteReassignment => "afc.channel:route-reassignment/1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelScope {
    pub provider: String,
    pub account: String,
    pub workspace: String,
    pub room: String,
    pub thread: Option<String>,
    pub sender: String,
}

impl ChannelScope {
    fn valid(&self) -> bool {
        [
            &self.provider,
            &self.account,
            &self.workspace,
            &self.room,
            &self.sender,
        ]
        .iter()
        .all(|value| !value.trim().is_empty())
            && self
                .thread
                .as_ref()
                .map_or(true, |value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelCausality {
    pub root_occurrence_id: String,
    pub parent_action_id: Option<Uuid>,
    pub action_id: Uuid,
    pub route_identity: String,
    pub visited_routes: Vec<String>,
    pub remaining_depth: u8,
    pub remaining_fanout: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelGrantSpec {
    pub issuer: String,
    pub grant_id: String,
    pub action: ChannelAction,
    pub scope: ChannelScope,
    pub recipient: String,
    pub handler: String,
    pub classification: String,
    pub max_remaining_depth: u8,
    pub max_remaining_fanout: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelGrantMutation {
    pub specification: ChannelGrantSpec,
    /// Absent only on creation; updates must compare Gate's current revision.
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelGrantState {
    pub specification: ChannelGrantSpec,
    pub revision: i64,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEffectRequest {
    pub protocol: String,
    pub effect_id: Uuid,
    pub occurrence_id: String,
    pub action: ChannelAction,
    pub scope: ChannelScope,
    pub recipient: String,
    pub handler: String,
    pub route_revision: String,
    pub payload: PayloadDigest,
    pub classification: String,
    pub causality: ChannelCausality,
    pub identity: VerifiedIdentity,
    /// Only the key is caller supplied. Gate resolves state and revision.
    pub grant_issuer: String,
    pub grant_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDisposition {
    Eligible,
    Released,
    Withheld,
    /// A release receipt already exists; the external effect may have run.
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEffectDecision {
    pub contract: String,
    pub protocol: String,
    pub receipt_id: Uuid,
    pub effect_id: Uuid,
    pub occurrence_id: String,
    pub action: ChannelAction,
    pub disposition: ChannelDisposition,
    pub reason: String,
    pub request_sha256: String,
    pub grant_revision: i64,
    pub policy_set_id: String,
    pub policy_revision: String,
    pub policy_digest: String,
}

#[derive(Clone)]
pub struct PostgresChannelAuthority {
    pool: PgPool,
}

impl PostgresChannelAuthority {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn put_grant(
        &self,
        mutation: ChannelGrantMutation,
        admin: AuthenticatedAdmin,
    ) -> Result<ChannelGrantState> {
        let spec = &mutation.specification;
        if admin.issuer != spec.issuer
            || admin.subject.trim().is_empty()
            || !spec.scope.valid()
            || [
                &spec.issuer,
                &spec.grant_id,
                &spec.recipient,
                &spec.handler,
                &spec.classification,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
            || spec.max_remaining_depth == 0
            || spec.max_remaining_depth > 4
            || spec.max_remaining_fanout == 0
            || spec.max_remaining_fanout > 8
        {
            bail!("invalid_channel_grant_scope_or_administrator");
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT revision FROM governed_channel_grants
             WHERE issuer = $1 AND grant_id = $2 FOR UPDATE",
        )
        .bind(&spec.issuer)
        .bind(&spec.grant_id)
        .fetch_optional(&mut *tx)
        .await?;
        let revision = match row {
            Some(row) => {
                let current: i64 = row.try_get("revision")?;
                if mutation.expected_revision != Some(current) {
                    bail!("channel_grant_revision_conflict");
                }
                current
                    .checked_add(1)
                    .context("channel grant revision overflow")?
            }
            None if mutation.expected_revision.is_none() => 1,
            None => bail!("channel_grant_not_found"),
        };
        let specification = serde_json::to_value(spec)?;
        let result = if revision == 1 {
            sqlx::query(
                "INSERT INTO governed_channel_grants
                 (issuer, grant_id, revision, active, specification, changed_by_issuer, changed_by_subject)
                 VALUES ($1, $2, $3, TRUE, $4, $5, $6)
                 ON CONFLICT (issuer, grant_id) DO NOTHING",
            )
            .bind(&spec.issuer)
            .bind(&spec.grant_id)
            .bind(revision)
            .bind(specification)
            .bind(&admin.issuer)
            .bind(&admin.subject)
            .execute(&mut *tx)
            .await?
        } else {
            sqlx::query(
                "UPDATE governed_channel_grants SET revision = $3, active = TRUE,
                 specification = $4, changed_by_issuer = $5,
                 changed_by_subject = $6, changed_at = NOW()
                 WHERE issuer = $1 AND grant_id = $2 AND revision = $7",
            )
            .bind(&spec.issuer)
            .bind(&spec.grant_id)
            .bind(revision)
            .bind(specification)
            .bind(&admin.issuer)
            .bind(&admin.subject)
            .bind(revision - 1)
            .execute(&mut *tx)
            .await?
        };
        if result.rows_affected() != 1 {
            bail!("channel_grant_revision_conflict");
        }
        tx.commit().await?;
        Ok(ChannelGrantState {
            specification: spec.clone(),
            revision,
            active: true,
        })
    }

    pub async fn revoke_grant(
        &self,
        issuer: &str,
        grant_id: &str,
        expected_revision: i64,
        admin: AuthenticatedAdmin,
    ) -> Result<Option<ChannelGrantState>> {
        if admin.issuer != issuer || admin.subject.trim().is_empty() || expected_revision < 1 {
            bail!("invalid_channel_grant_revoke_authority");
        }
        let row = sqlx::query(
            "UPDATE governed_channel_grants
             SET revision = revision + 1, active = FALSE,
                 changed_by_issuer = $4, changed_by_subject = $5, changed_at = NOW()
             WHERE issuer = $1 AND grant_id = $2 AND revision = $3
             RETURNING specification, revision, active",
        )
        .bind(issuer)
        .bind(grant_id)
        .bind(expected_revision)
        .bind(&admin.issuer)
        .bind(&admin.subject)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ChannelGrantState {
                specification: serde_json::from_value(row.try_get("specification")?)?,
                revision: row.try_get("revision")?,
                active: row.try_get("active")?,
            })
        })
        .transpose()
    }

    async fn locked_grant(
        tx: &mut Transaction<'_, Postgres>,
        issuer: &str,
        grant_id: &str,
    ) -> Result<Option<ChannelGrantState>> {
        let row = sqlx::query(
            "SELECT specification, revision, active FROM governed_channel_grants
             WHERE issuer = $1 AND grant_id = $2 FOR SHARE",
        )
        .bind(issuer)
        .bind(grant_id)
        .fetch_optional(&mut **tx)
        .await?;
        row.map(|row| {
            Ok(ChannelGrantState {
                specification: serde_json::from_value(row.try_get("specification")?)?,
                revision: row.try_get("revision")?,
                active: row.try_get("active")?,
            })
        })
        .transpose()
    }

    fn invalid_reason(request: &ChannelEffectRequest) -> Option<&'static str> {
        if request.protocol != CHANNEL_PROTOCOL {
            return Some("unsupported_channel_protocol");
        }
        if request.effect_id.is_nil()
            || request.causality.action_id.is_nil()
            || !request.scope.valid()
            || [
                &request.occurrence_id,
                &request.recipient,
                &request.handler,
                &request.route_revision,
                &request.classification,
                &request.grant_issuer,
                &request.grant_id,
                &request.causality.root_occurrence_id,
                &request.causality.route_identity,
            ]
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Some("invalid_channel_scope_or_route");
        }
        if request.payload.algorithm != "sha256"
            || request.payload.sha256.len() != 64
            || !request
                .payload
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Some("invalid_channel_payload_digest");
        }
        if !request.identity.verified
            || request.identity.revoked
            || request.identity.issuer.trim().is_empty()
            || request.identity.subject.trim().is_empty()
            || request.identity.identity_revision.trim().is_empty()
        {
            return Some("invalid_channel_identity");
        }
        // These counters describe capacity remaining after the current action
        // was admitted. Zero therefore represents a valid terminal effect;
        // grant maxima reject over-budget values below, while route history
        // independently rejects cycles.
        if request
            .causality
            .visited_routes
            .contains(&request.causality.route_identity)
        {
            return Some("channel_causal_budget_exhausted_or_route_repeated");
        }
        None
    }

    fn grant_reason(
        request: &ChannelEffectRequest,
        grant: Option<&ChannelGrantState>,
    ) -> Option<&'static str> {
        let Some(grant) = grant else {
            return Some("channel_grant_not_found");
        };
        if !grant.active {
            return Some("channel_grant_revoked");
        }
        let spec = &grant.specification;
        if spec.issuer != request.grant_issuer
            || spec.grant_id != request.grant_id
            || spec.action != request.action
            || spec.scope.provider != request.scope.provider
            || spec.scope.account != request.scope.account
            || spec.scope.workspace != request.scope.workspace
            || spec.scope.room != request.scope.room
            || spec.scope.thread != request.scope.thread
            || (spec.scope.sender != "*" && spec.scope.sender != request.scope.sender)
            || spec.recipient != request.recipient
            || spec.handler != request.handler
            || spec.classification != request.classification
        {
            return Some("channel_grant_scope_mismatch");
        }
        if request.causality.remaining_depth > spec.max_remaining_depth
            || request.causality.remaining_fanout > spec.max_remaining_fanout
        {
            return Some("channel_grant_causal_budget_exceeded");
        }
        None
    }

    fn policy_reason(authz: &AuthzEngine, request: &ChannelEffectRequest) -> Option<&'static str> {
        let resource = serde_json::to_string(&(
            &request.scope,
            &request.recipient,
            &request.handler,
            &request.route_revision,
        ))
        .unwrap_or_default();
        let mut context = serde_json::to_value(request).unwrap_or_default();
        Self::remove_null_context_values(&mut context);
        match authz.authorize_as(
            request.identity.subject_kind.into(),
            &request.identity.subject,
            request.action.cedar_action(),
            &resource,
            &context,
        ) {
            AuthzDecision::Allow => None,
            AuthzDecision::Deny => Some("channel_policy_denied"),
            AuthzDecision::RequireApproval(_) => Some("channel_policy_approval_required"),
        }
    }

    fn remove_null_context_values(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                fields.retain(|_, field| !field.is_null());
                fields
                    .values_mut()
                    .for_each(Self::remove_null_context_values);
            }
            serde_json::Value::Array(items) => {
                items.retain(|item| !item.is_null());
                items
                    .iter_mut()
                    .for_each(Self::remove_null_context_values);
            }
            _ => {}
        }
    }

    fn request_digest(
        request: &ChannelEffectRequest,
        owner: &ExecutionOwnerAttestation,
    ) -> Result<String> {
        // Bind the exact Cedar input. Case-folding the payload hex here while
        // Cedar sees the original bytes would permit two policy inputs to
        // share one effect identity.
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&(
            request, owner,
        ))?)))
    }

    /// Evaluate and persist a receipt. This never authorizes queue release.
    pub async fn evaluate(
        &self,
        authz: &Arc<AuthzEngine>,
        request: ChannelEffectRequest,
        owner: ExecutionOwnerAttestation,
    ) -> Result<ChannelEffectDecision> {
        self.decide(authz, request, owner, false).await
    }

    /// Recheck current grant and Cedar authority in the same transaction as
    /// marking this exact receipt releasable. The caller owns execution.
    pub async fn release(
        &self,
        authz: &Arc<AuthzEngine>,
        request: ChannelEffectRequest,
        owner: ExecutionOwnerAttestation,
    ) -> Result<ChannelEffectDecision> {
        self.decide(authz, request, owner, true).await
    }

    async fn decide(
        &self,
        authz: &Arc<AuthzEngine>,
        request: ChannelEffectRequest,
        owner: ExecutionOwnerAttestation,
        release: bool,
    ) -> Result<ChannelEffectDecision> {
        let request_sha256 = Self::request_digest(&request, &owner)?;
        let mut tx = self.pool.begin().await?;
        let grant = Self::locked_grant(&mut tx, &request.grant_issuer, &request.grant_id).await?;
        let snapshot = authz.pinned_snapshot();
        let policy = snapshot.active_policy_set();
        let policy_digest = policy.digest.clone();
        let reason = if owner.issuer != request.grant_issuer
            || owner.subject.trim().is_empty()
            || owner.fact_source.trim().is_empty()
        {
            Some("untrusted_channel_execution_owner")
        } else {
            Self::invalid_reason(&request)
                .or_else(|| Self::grant_reason(&request, grant.as_ref()))
                .or_else(|| Self::policy_reason(&snapshot, &request))
        };
        let evaluation = sqlx::query(
            "SELECT receipt_id, request_sha256, disposition, grant_revision,
                    policy_set_id, policy_revision, policy_digest
             FROM governed_channel_effect_receipts
             WHERE effect_id = $1 AND stage = 'evaluate' FOR UPDATE",
        )
        .bind(request.effect_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &evaluation {
            let recorded_hash: String = row.try_get("request_sha256")?;
            if recorded_hash != request_sha256 {
                bail!("channel_effect_identity_collision");
            }
        }
        let stage = if release { "release" } else { "evaluate" };
        let stage_record = if release {
            sqlx::query(
                "SELECT receipt_id, request_sha256, disposition FROM governed_channel_effect_receipts
                 WHERE effect_id = $1 AND stage = 'release' FOR UPDATE",
            )
            .bind(request.effect_id)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            None
        };
        if let Some(row) = &stage_record {
            let recorded_hash: String = row.try_get("request_sha256")?;
            if recorded_hash != request_sha256 {
                bail!("channel_effect_identity_collision");
            }
        }
        let receipt_id = if release {
            stage_record
                .as_ref()
                .map(|row| row.try_get("receipt_id"))
                .transpose()?
                .unwrap_or_else(Uuid::new_v4)
        } else {
            evaluation
                .as_ref()
                .map(|row| row.try_get("receipt_id"))
                .transpose()?
                .unwrap_or_else(Uuid::new_v4)
        };
        let revision = grant.as_ref().map_or(0, |grant| grant.revision);
        let previous_revision = evaluation
            .as_ref()
            .map(|row| row.try_get::<i64, _>("grant_revision"))
            .transpose()?;
        let previous_policy = evaluation
            .as_ref()
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.try_get::<String, _>("policy_set_id")?,
                    row.try_get::<String, _>("policy_revision")?,
                    row.try_get::<String, _>("policy_digest")?,
                ))
            })
            .transpose()?;
        let reason = reason.or_else(|| {
            (evaluation.is_some() && previous_revision != Some(revision))
                .then_some("channel_grant_revision_changed_since_evaluation")
        });
        let reason = reason.or_else(|| {
            (evaluation.is_some()
                && previous_policy.as_ref()
                    != Some(&(
                        policy.set_id.clone(),
                        policy.revision.clone(),
                        policy_digest.clone(),
                    )))
            .then_some("channel_policy_changed_since_evaluation")
        });
        let reason = reason.or_else(|| {
            (release && evaluation.is_none()).then_some("channel_effect_not_evaluated")
        });
        let reason = reason.or_else(|| {
            (release
                && evaluation.as_ref().is_some_and(|row| {
                    row.try_get::<String, _>("disposition")
                        .is_ok_and(|value| value != "eligible")
                }))
            .then_some("channel_effect_evaluation_withheld")
        });
        let reason = reason.or_else(|| {
            (release && authz.active_policy_set() != policy)
                .then_some("channel_policy_changed_before_release")
        });
        let reason = reason.or_else(|| {
            (release && stage_record.is_some())
                .then_some("channel_effect_release_receipt_already_exists")
        });
        let stage_released = stage_record.as_ref().is_some_and(|row| {
            row.try_get::<String, _>("disposition")
                .is_ok_and(|value| value == "released")
        });
        let disposition = if stage_released {
            ChannelDisposition::Uncertain
        } else if stage_record.is_some() {
            ChannelDisposition::Withheld
        } else if reason.is_some() {
            ChannelDisposition::Withheld
        } else if release {
            ChannelDisposition::Released
        } else {
            ChannelDisposition::Eligible
        };
        let reason = reason.unwrap_or(if release {
            "channel_effect_revalidated_for_release"
        } else {
            "channel_effect_eligible_pending_release"
        });
        if stage_record.is_none() && (release || evaluation.is_none()) {
            let result = sqlx::query(
                "INSERT INTO governed_channel_effect_receipts
                 (receipt_id, effect_id, action_id, stage, issuer, grant_id, grant_revision, action,
                  occurrence_id, request_sha256, policy_set_id, policy_revision,
                  policy_digest, disposition, reason, released_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                         CASE WHEN $14 = 'released' THEN NOW() ELSE NULL END)
                 ON CONFLICT DO NOTHING",
            )
            .bind(receipt_id)
            .bind(request.effect_id)
            .bind(request.causality.action_id)
            .bind(stage)
            .bind(&request.grant_issuer)
            .bind(&request.grant_id)
            .bind(revision)
            .bind(request.action.cedar_action())
            .bind(&request.occurrence_id)
            .bind(&request_sha256)
            .bind(&policy.set_id)
            .bind(&policy.revision)
            .bind(&policy_digest)
            .bind(match disposition {
                ChannelDisposition::Eligible => "eligible",
                ChannelDisposition::Released => "released",
                ChannelDisposition::Withheld => "withheld",
                ChannelDisposition::Uncertain => "withheld",
            })
            .bind(reason)
            .execute(&mut *tx)
            .await
            .context("persisting channel effect receipt")?;
            if result.rows_affected() != 1 {
                bail!("channel_effect_identity_collision");
            }
        }
        tx.commit().await?;
        Ok(ChannelEffectDecision {
            contract: CHANNEL_AUTHORITY_CONTRACT.to_owned(),
            protocol: CHANNEL_PROTOCOL.to_owned(),
            receipt_id,
            effect_id: request.effect_id,
            occurrence_id: request.occurrence_id,
            action: request.action,
            disposition,
            reason: if disposition == ChannelDisposition::Uncertain {
                "channel_effect_release_receipt_already_exists_outcome_uncertain".to_owned()
            } else {
                reason.to_owned()
            },
            request_sha256,
            grant_revision: revision,
            policy_set_id: policy.set_id,
            policy_revision: policy.revision,
            policy_digest,
        })
    }
}
