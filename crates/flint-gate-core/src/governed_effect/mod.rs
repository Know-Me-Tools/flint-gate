//! Transport-neutral governed-effect authority and durable approval contract.
//!
//! This module decides whether an already-described effect may run. It never
//! executes, schedules, or retries the effect.

mod postgres;

pub use postgres::PostgresChallengeStore;

use crate::authz::{AuthzDecision, AuthzEngine, PrincipalKind};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "afc.governed-effect/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    User,
    Agent,
    Service,
}

impl From<SubjectKind> for PrincipalKind {
    fn from(value: SubjectKind) -> Self {
        match value {
            SubjectKind::User => Self::User,
            SubjectKind::Agent => Self::Agent,
            SubjectKind::Service => Self::Service,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualifiedAction {
    pub namespace: String,
    pub name: String,
    pub version: String,
}

impl QualifiedAction {
    fn canonical(&self) -> String {
        format!("{}:{}/{}", self.namespace, self.name, self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalResource {
    pub kind: String,
    pub destination: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadDigest {
    pub algorithm: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub subject_kind: SubjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    pub identity_revision: String,
    pub verified: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRef {
    pub set_id: String,
    pub revision: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRef {
    pub issuer: String,
    pub grant_id: String,
    pub revision: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRef {
    pub lease_id: String,
    pub revision: String,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetRef {
    pub budget_id: String,
    pub revision: String,
    pub reservation_id: String,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityEpochs {
    pub runtime: String,
    pub host: String,
    pub catalog: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedEffectRequest {
    pub protocol: String,
    pub effect_id: Uuid,
    pub invocation_id: Uuid,
    pub action: QualifiedAction,
    pub resource: CanonicalResource,
    pub payload: PayloadDigest,
    pub identity: VerifiedIdentity,
    #[serde(default)]
    pub grants: Vec<GrantRef>,
    pub lease: LeaseRef,
    pub budget: BudgetRef,
    pub epochs: AuthorityEpochs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBinding {
    pub request_sha256: String,
    pub policy: PolicyRef,
    pub execution_owner: ExecutionOwnerAttestation,
    pub grants: Vec<GrantRef>,
    pub lease: LeaseRef,
    pub budget: BudgetRef,
    pub identity_revision: String,
    pub epochs: AuthorityEpochs,
}

/// Provenance applied by Gate after authenticating the P1 execution owner.
/// Lease and budget facts are accepted only under this attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOwnerAttestation {
    pub fact_source: String,
    pub issuer: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDisposition {
    Deny,
    Challenge,
    Permit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedEffectDecision {
    pub protocol: String,
    pub disposition: EffectDisposition,
    pub effect_id: Uuid,
    pub invocation_id: Uuid,
    pub binding: AuthorityBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_id: Option<Uuid>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeDecision {
    Approve,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedAdmin {
    pub issuer: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeRecord {
    pub issuer: String,
    pub challenge_id: Uuid,
    pub request: GovernedEffectRequest,
    pub binding: AuthorityBinding,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ChallengeDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<AuthenticatedAdmin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevalidateRequest {
    pub issuer: String,
    pub challenge_id: Uuid,
    pub request: GovernedEffectRequest,
}

#[async_trait]
pub trait ChallengeStore: Send + Sync {
    async fn register(&self, challenge: ChallengeRecord) -> Result<()>;
    async fn load(&self, issuer: &str, challenge_id: Uuid) -> Result<Option<ChallengeRecord>>;
    async fn decide(
        &self,
        issuer: &str,
        challenge_id: Uuid,
        decision: ChallengeDecision,
        admin: &AuthenticatedAdmin,
    ) -> Result<bool>;
}

#[derive(Default)]
pub struct MemoryChallengeStore {
    records: DashMap<(String, Uuid), ChallengeRecord>,
}

impl MemoryChallengeStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl ChallengeStore for MemoryChallengeStore {
    async fn register(&self, challenge: ChallengeRecord) -> Result<()> {
        let key = (challenge.issuer.clone(), challenge.challenge_id);
        match self.records.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(entry)
                if entry.get().binding == challenge.binding =>
            {
                Ok(())
            }
            dashmap::mapref::entry::Entry::Occupied(_) => Err(anyhow::anyhow!(
                "governed-effect challenge identity collision"
            )),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(challenge);
                Ok(())
            }
        }
    }

    async fn load(&self, issuer: &str, challenge_id: Uuid) -> Result<Option<ChallengeRecord>> {
        Ok(self
            .records
            .get(&(issuer.to_owned(), challenge_id))
            .map(|record| record.clone()))
    }

    async fn decide(
        &self,
        issuer: &str,
        challenge_id: Uuid,
        decision: ChallengeDecision,
        admin: &AuthenticatedAdmin,
    ) -> Result<bool> {
        let Some(mut record) = self.records.get_mut(&(issuer.to_owned(), challenge_id)) else {
            return Ok(false);
        };
        if record.decision.is_some() || record.expires_at <= Utc::now() {
            return Ok(false);
        }
        record.decision = Some(decision);
        record.decided_at = Some(Utc::now());
        record.decided_by = Some(admin.clone());
        Ok(true)
    }
}

#[async_trait]
pub trait GovernedEffectAuthorityProvider: Send + Sync {
    async fn evaluate(
        &self,
        request: GovernedEffectRequest,
        execution_owner: ExecutionOwnerAttestation,
    ) -> Result<GovernedEffectDecision>;
    async fn revalidate(
        &self,
        request: RevalidateRequest,
        execution_owner: ExecutionOwnerAttestation,
    ) -> Result<GovernedEffectDecision>;
    async fn decide(
        &self,
        issuer: &str,
        challenge_id: Uuid,
        decision: ChallengeDecision,
        admin: AuthenticatedAdmin,
    ) -> Result<bool>;
}

pub struct CedarGovernedEffectAuthority {
    authz: Arc<AuthzEngine>,
    challenges: Arc<dyn ChallengeStore>,
    challenge_ttl: chrono::Duration,
}

impl CedarGovernedEffectAuthority {
    pub fn new(authz: Arc<AuthzEngine>, challenges: Arc<dyn ChallengeStore>) -> Self {
        Self {
            authz,
            challenges,
            challenge_ttl: chrono::Duration::minutes(5),
        }
    }

    fn binding(
        authz: &AuthzEngine,
        request: &GovernedEffectRequest,
        execution_owner: ExecutionOwnerAttestation,
    ) -> Result<AuthorityBinding> {
        let mut canonical = request.clone();
        canonical.payload.sha256.make_ascii_lowercase();
        canonical.grants.sort_by(|left, right| {
            (&left.issuer, &left.grant_id, &left.revision).cmp(&(
                &right.issuer,
                &right.grant_id,
                &right.revision,
            ))
        });
        if let Some(audience) = canonical.identity.audience.as_mut() {
            audience.sort();
        }
        let bytes = serde_json::to_vec(&canonical)?;
        let active_policy = authz.active_policy_set();
        Ok(AuthorityBinding {
            request_sha256: hex::encode(Sha256::digest(bytes)),
            policy: PolicyRef {
                set_id: active_policy.set_id,
                revision: active_policy.revision,
                digest: active_policy.digest,
            },
            execution_owner,
            grants: canonical.grants,
            lease: canonical.lease,
            budget: canonical.budget,
            identity_revision: canonical.identity.identity_revision,
            epochs: canonical.epochs,
        })
    }

    fn invalid_reason(request: &GovernedEffectRequest) -> Option<&'static str> {
        if request.protocol != PROTOCOL_VERSION {
            return Some("unsupported_protocol");
        }
        if request.action.namespace.trim().is_empty()
            || request.action.name.trim().is_empty()
            || request.action.version.trim().is_empty()
            || request.resource.kind.trim().is_empty()
            || request.resource.destination.trim().is_empty()
        {
            return Some("invalid_action_or_resource");
        }
        if request.payload.algorithm != "sha256"
            || request.payload.sha256.len() != 64
            || !request.payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Some("invalid_payload_digest");
        }
        if !request.identity.verified
            || request.identity.revoked
            || request.identity.issuer.trim().is_empty()
            || request.identity.subject.trim().is_empty()
            || request.identity.identity_revision.trim().is_empty()
        {
            return Some("invalid_or_revoked_identity");
        }
        if request
            .identity
            .actor
            .as_ref()
            .is_some_and(|actor| actor.trim().is_empty())
            || request.identity.audience.as_ref().is_some_and(|audience| {
                audience.is_empty()
                    || audience.iter().any(|value| value.trim().is_empty())
                    || audience
                        .iter()
                        .enumerate()
                        .any(|(index, value)| audience[..index].contains(value))
            })
            || request
                .identity
                .tenant
                .as_ref()
                .is_some_and(|tenant| tenant.trim().is_empty())
        {
            return Some("invalid_identity_scope");
        }
        if request.grants.iter().any(|grant| {
            !grant.active
                || grant.issuer.trim().is_empty()
                || grant.issuer != request.identity.issuer
                || grant.grant_id.trim().is_empty()
                || grant.revision.trim().is_empty()
        }) {
            return Some("inactive_or_invalid_grant");
        }
        if !request.lease.active
            || request.lease.expires_at <= Utc::now()
            || request.lease.lease_id.trim().is_empty()
            || request.lease.revision.trim().is_empty()
        {
            return Some("inactive_or_expired_lease");
        }
        if !request.budget.active
            || request.budget.expires_at <= Utc::now()
            || request.budget.budget_id.trim().is_empty()
            || request.budget.revision.trim().is_empty()
            || request.budget.reservation_id.trim().is_empty()
        {
            return Some("inactive_expired_or_invalid_budget");
        }
        if request.epochs.runtime.trim().is_empty()
            || request.epochs.host.trim().is_empty()
            || request.epochs.catalog.trim().is_empty()
        {
            return Some("invalid_authority_epoch");
        }
        None
    }

    fn execution_owner_is_trusted(attestation: &ExecutionOwnerAttestation) -> bool {
        !attestation.fact_source.trim().is_empty()
            && !attestation.issuer.trim().is_empty()
            && !attestation.subject.trim().is_empty()
    }

    fn cedar_decision(authz: &AuthzEngine, request: &GovernedEffectRequest) -> AuthzDecision {
        let context = serde_json::to_value(request).unwrap_or(Value::Null);
        authz.authorize_as(
            request.identity.subject_kind.into(),
            &request.identity.subject,
            &request.action.canonical(),
            &request.resource.destination,
            &context,
        )
    }

    fn challenge_id(request: &GovernedEffectRequest, binding: &AuthorityBinding) -> Uuid {
        let mut digest = Sha256::new();
        digest.update(request.identity.issuer.as_bytes());
        digest.update(request.effect_id.as_bytes());
        digest.update(request.invocation_id.as_bytes());
        digest.update(binding.request_sha256.as_bytes());
        digest.update(binding.policy.digest.as_bytes());
        digest.update(binding.execution_owner.fact_source.as_bytes());
        digest.update(binding.execution_owner.issuer.as_bytes());
        digest.update(binding.execution_owner.subject.as_bytes());
        let bytes = digest.finalize();
        let mut id = [0_u8; 16];
        id.copy_from_slice(&bytes[..16]);
        id[6] = (id[6] & 0x0f) | 0x50;
        id[8] = (id[8] & 0x3f) | 0x80;
        Uuid::from_bytes(id)
    }

    fn decision(
        request: &GovernedEffectRequest,
        binding: AuthorityBinding,
        disposition: EffectDisposition,
        challenge_id: Option<Uuid>,
        reason: impl Into<String>,
    ) -> GovernedEffectDecision {
        GovernedEffectDecision {
            protocol: PROTOCOL_VERSION.to_owned(),
            disposition,
            effect_id: request.effect_id,
            invocation_id: request.invocation_id,
            binding,
            challenge_id,
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl GovernedEffectAuthorityProvider for CedarGovernedEffectAuthority {
    async fn evaluate(
        &self,
        request: GovernedEffectRequest,
        execution_owner: ExecutionOwnerAttestation,
    ) -> Result<GovernedEffectDecision> {
        let authz = self.authz.pinned_snapshot();
        let invalid_reason = Self::invalid_reason(&request);
        let binding = Self::binding(&authz, &request, execution_owner)?;
        if !Self::execution_owner_is_trusted(&binding.execution_owner) {
            return Ok(Self::decision(
                &request,
                binding,
                EffectDisposition::Deny,
                None,
                "untrusted_execution_owner_attestation",
            ));
        }
        if let Some(reason) = invalid_reason {
            return Ok(Self::decision(
                &request,
                binding,
                EffectDisposition::Deny,
                None,
                reason,
            ));
        }
        match Self::cedar_decision(&authz, &request) {
            AuthzDecision::Deny => Ok(Self::decision(
                &request,
                binding,
                EffectDisposition::Deny,
                None,
                "policy_denied",
            )),
            AuthzDecision::Allow => Ok(Self::decision(
                &request,
                binding,
                EffectDisposition::Permit,
                None,
                "policy_permitted",
            )),
            AuthzDecision::RequireApproval(_) => {
                let challenge_id = Self::challenge_id(&request, &binding);
                let now = Utc::now();
                self.challenges
                    .register(ChallengeRecord {
                        issuer: request.identity.issuer.clone(),
                        challenge_id,
                        request: request.clone(),
                        binding: binding.clone(),
                        created_at: now,
                        expires_at: now + self.challenge_ttl,
                        decision: None,
                        decided_at: None,
                        decided_by: None,
                    })
                    .await?;
                Ok(Self::decision(
                    &request,
                    binding,
                    EffectDisposition::Challenge,
                    Some(challenge_id),
                    "approval_required",
                ))
            }
        }
    }

    async fn revalidate(
        &self,
        input: RevalidateRequest,
        execution_owner: ExecutionOwnerAttestation,
    ) -> Result<GovernedEffectDecision> {
        let authz = self.authz.pinned_snapshot();
        let invalid_reason = Self::invalid_reason(&input.request);
        let binding = Self::binding(&authz, &input.request, execution_owner)?;
        if !Self::execution_owner_is_trusted(&binding.execution_owner) {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "untrusted_execution_owner_attestation",
            ));
        }
        if let Some(reason) = invalid_reason {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                reason,
            ));
        }
        let Some(challenge) = self.challenges.load(&input.issuer, input.challenge_id).await? else {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "approval_not_found",
            ));
        };
        if input.issuer != input.request.identity.issuer
            || challenge.issuer != input.request.identity.issuer
            || challenge.binding != binding
        {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "authority_binding_changed",
            ));
        }
        if challenge.expires_at <= Utc::now() {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "approval_expired",
            ));
        }
        if challenge.decision != Some(ChallengeDecision::Approve) {
            return Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "approval_not_granted",
            ));
        }
        match Self::cedar_decision(&authz, &input.request) {
            AuthzDecision::Deny => Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Deny,
                None,
                "policy_denied_after_wait",
            )),
            AuthzDecision::Allow | AuthzDecision::RequireApproval(_) => Ok(Self::decision(
                &input.request,
                binding,
                EffectDisposition::Permit,
                None,
                "approved_and_revalidated",
            )),
        }
    }

    async fn decide(
        &self,
        issuer: &str,
        challenge_id: Uuid,
        decision: ChallengeDecision,
        admin: AuthenticatedAdmin,
    ) -> Result<bool> {
        if issuer.trim().is_empty()
            || admin.issuer.trim().is_empty()
            || admin.subject.trim().is_empty()
        {
            return Ok(false);
        }
        self.challenges
            .decide(issuer, challenge_id, decision, &admin)
            .await
    }
}
