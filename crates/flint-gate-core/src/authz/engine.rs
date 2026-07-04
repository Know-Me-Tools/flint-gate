//! The shared authorization engine: a lock-free-on-read Cedar evaluator.

use std::str::FromStr;
use std::sync::Arc;

use arc_swap::ArcSwap;
use cedar_policy::{
    Authorizer, Context, Decision, Effect, EntityId, EntityTypeName, EntityUid, PolicyId, Request,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, warn};
use uuid::Uuid;

use super::bundle::{CedarBundle, PolicyRecord};
use super::error::AuthzError;

/// Default Cedar entity type used for the request principal (human user).
const PRINCIPAL_TYPE: &str = "User";

/// The Cedar principal **entity type** a request authorizes as. Lets a policy
/// distinguish human users from non-human identities — e.g.
/// `permit(principal == Agent::"bot-7", …)` vs `principal == User::"alice"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    /// A human user → Cedar `User::"<id>"` (the default; backward compatible).
    #[default]
    User,
    /// A delegated agent identity → Cedar `Agent::"<id>"`.
    Agent,
    /// A service / workload identity → Cedar `Service::"<id>"`.
    Service,
}

impl PrincipalKind {
    /// The Cedar entity type name for this principal kind.
    pub fn cedar_type(self) -> &'static str {
        match self {
            PrincipalKind::User => PRINCIPAL_TYPE,
            PrincipalKind::Agent => "Agent",
            PrincipalKind::Service => "Service",
        }
    }
}
/// Cedar entity type used for the request action.
const ACTION_TYPE: &str = "Action";
/// Cedar entity type used for the request resource (a route).
const RESOURCE_TYPE: &str = "Route";
/// Default generic action id when a hook does not specify one.
pub const DEFAULT_ACTION: &str = "invoke";
/// Default time-to-live for a pending approval request (5 minutes).
pub const DEFAULT_APPROVAL_TTL_SECONDS: i64 = 300;

/// Context carried when an authorization decision requires human approval.
///
/// The engine produces this for a Cedar `Allow` matched by a policy bearing the
/// `@require_approval` annotation. Higher layers (e.g. the streaming tool-authz
/// path) may augment the context with request-specific details such as the
/// tool-call id before emitting it to a frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalContext {
    /// Stable, unique identifier for this approval request.
    pub approval_id: String,
    /// Principal that initiated the request.
    pub principal_id: String,
    /// Action being authorized.
    pub action: String,
    /// Resource being accessed.
    pub resource_id: String,
    /// Human-readable reason from the policy annotation, if any.
    pub reason: Option<String>,
    /// UTC timestamp after which the approval request expires.
    pub expires_at: DateTime<Utc>,
}

impl ApprovalContext {
    /// Build an approval context with a freshly generated id and default TTL.
    pub fn new(
        principal_id: impl Into<String>,
        action: impl Into<String>,
        resource_id: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        Self {
            approval_id: Uuid::new_v4().to_string(),
            principal_id: principal_id.into(),
            action: action.into(),
            resource_id: resource_id.into(),
            reason,
            expires_at: Utc::now() + Duration::seconds(DEFAULT_APPROVAL_TTL_SECONDS),
        }
    }

    /// Build an approval context with an explicit id and expiry.
    pub fn with_id_and_expiry(
        approval_id: impl Into<String>,
        principal_id: impl Into<String>,
        action: impl Into<String>,
        resource_id: impl Into<String>,
        reason: Option<String>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            approval_id: approval_id.into(),
            principal_id: principal_id.into(),
            action: action.into(),
            resource_id: resource_id.into(),
            reason,
            expires_at,
        }
    }

    /// Has this approval request expired?
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// The outcome of an authorization check.
///
/// Three outcomes are possible: allow, deny, or require human approval. Every
/// error, ambiguity, or construction failure collapses to [`Self::Deny`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzDecision {
    /// The request is permitted.
    Allow,
    /// The request is denied (explicitly, or fail-closed on any error).
    Deny,
    /// The request requires human approval before proceeding.
    RequireApproval(ApprovalContext),
}

impl AuthzDecision {
    /// Is this an unconditional allow?
    pub fn is_allow(&self) -> bool {
        matches!(self, AuthzDecision::Allow)
    }

    /// Is this a denial?
    pub fn is_deny(&self) -> bool {
        matches!(self, AuthzDecision::Deny)
    }

    /// Is this a require-approval decision?
    pub fn is_require_approval(&self) -> bool {
        matches!(self, AuthzDecision::RequireApproval(_))
    }

    /// If this is a require-approval decision, return the approval context.
    pub fn approval_context(&self) -> Option<&ApprovalContext> {
        match self {
            AuthzDecision::RequireApproval(ctx) => Some(ctx),
            _ => None,
        }
    }
}

/// The embedded authorization engine.
///
/// Holds the live [`CedarBundle`] behind an [`ArcSwap`] so readers on the
/// request path load a consistent snapshot without a lock, while a background
/// reload can atomically replace it.
pub struct AuthzEngine {
    bundle: ArcSwap<CedarBundle>,
    authorizer: Authorizer,
}

impl std::fmt::Debug for AuthzEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthzEngine")
            .field("bundle", &self.bundle.load())
            .finish()
    }
}

impl AuthzEngine {
    /// Create an engine around an already-compiled bundle.
    pub fn new(bundle: CedarBundle) -> Self {
        Self {
            bundle: ArcSwap::from_pointee(bundle),
            authorizer: Authorizer::new(),
        }
    }

    /// Create an engine with an empty (default-deny) bundle. This is the
    /// fail-closed startup state when no policies are configured.
    pub fn empty() -> Self {
        Self::new(CedarBundle::empty())
    }

    /// Build an engine directly from policy records.
    pub fn from_records(records: &[PolicyRecord]) -> Result<Self, AuthzError> {
        Ok(Self::new(CedarBundle::from_records(records)?))
    }

    /// Load the current snapshot of the bundle (lock-free).
    pub fn snapshot(&self) -> Arc<CedarBundle> {
        self.bundle.load_full()
    }

    /// Load enabled policies from the database and build the initial bundle.
    ///
    /// Uses the LENIENT loader: individual poisoned rows are skipped (logged),
    /// and the engine is built from the survivors, so one bad stored policy can
    /// never black-hole all authorization. A DB-load failure logs and returns an
    /// EMPTY (default-deny) engine — the safe fail-closed floor. When the
    /// database is absent the engine is empty (deny-all) by construction.
    pub async fn from_database(db: &crate::db::Database) -> Self {
        match db.load_enabled_policies().await {
            Ok(rows) => {
                let records: Vec<PolicyRecord> =
                    rows.into_iter().map(|r| r.into_record()).collect();
                Self::new(CedarBundle::from_records_lenient(&records))
            }
            Err(e) => {
                error!(error = %e, "failed to load policies at startup — starting default-deny");
                Self::empty()
            }
        }
    }

    /// Reload from the database: parse-before-swap, fail-closed (retain
    /// last-good on a DB-load failure). Individual poisoned rows are skipped via
    /// the lenient loader so one bad row (possibly written by another replica)
    /// cannot black-hole a peer's authorization. Returns the DB error, if any,
    /// so callers can surface a load failure.
    pub async fn reload_from_database(&self, db: &crate::db::Database) -> Result<(), AuthzError> {
        let rows = db
            .load_enabled_policies()
            .await
            .map_err(|e| AuthzError::Load(e.to_string()))?;
        let records: Vec<PolicyRecord> = rows.into_iter().map(|r| r.into_record()).collect();
        // Lenient parse-before-swap: build the survivors' bundle, then store.
        self.reload_from_records_lenient(&records);
        Ok(())
    }

    /// Lenient parse-before-swap reload from in-memory records. Poisoned rows
    /// are skipped (logged) and the engine is rebuilt from the survivors, then
    /// the new bundle is atomically stored. This is the shared core of
    /// [`Self::reload_from_database`] and is directly testable without a live DB
    /// (it is what a "policies" NOTIFY drives on every replica — C1).
    pub fn reload_from_records_lenient(&self, records: &[PolicyRecord]) {
        let new_bundle = CedarBundle::from_records_lenient(records);
        self.bundle.store(Arc::new(new_bundle));
    }

    /// Parse-before-swap reload. The new bundle is compiled FIRST; only if that
    /// succeeds is it atomically stored. On any parse/validation failure the
    /// last-good bundle is RETAINED and the error is returned — a bad reload can
    /// never blank the policy set.
    pub fn reload_from_records(&self, records: &[PolicyRecord]) -> Result<(), AuthzError> {
        match CedarBundle::from_records(records) {
            Ok(new_bundle) => {
                self.bundle.store(Arc::new(new_bundle));
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "authz reload failed — retaining last-good policy bundle");
                Err(e)
            }
        }
    }

    /// Authorize a request. Fail-closed: any error yields [`AuthzDecision::Deny`].
    ///
    /// - `principal_id` → `User::"<id>"`
    /// - `action` → `Action::"<action>"` (generic, e.g. `invoke`)
    /// - `resource_id` → `Route::"<id>"`
    /// - `context` → a JSON object mapped into the Cedar request context
    ///
    /// When a matching `permit` policy carries the `@require_approval`
    /// annotation, an otherwise-allowing decision is promoted to
    /// [`AuthzDecision::RequireApproval`].
    pub fn authorize(
        &self,
        principal_id: &str,
        action: &str,
        resource_id: &str,
        context: &Value,
    ) -> AuthzDecision {
        self.authorize_as(PrincipalKind::User, principal_id, action, resource_id, context)
    }

    /// Authorize a request with an explicit principal **kind** (User / Agent /
    /// Service), so a policy can name a non-human identity as principal. Same
    /// fail-closed semantics as [`Self::authorize`].
    pub fn authorize_as(
        &self,
        kind: PrincipalKind,
        principal_id: &str,
        action: &str,
        resource_id: &str,
        context: &Value,
    ) -> AuthzDecision {
        let snapshot = self.bundle.load();
        match self.evaluate(&snapshot, kind, principal_id, action, resource_id, context) {
            Ok((Decision::Allow, Some(ctx))) => AuthzDecision::RequireApproval(ctx),
            Ok((Decision::Allow, None)) => AuthzDecision::Allow,
            Ok((Decision::Deny, _)) => AuthzDecision::Deny,
            Err(e) => {
                // Fail closed. A malformed principal id, un-mappable context, or
                // schema-mismatched request must never fall through to allow.
                warn!(
                    error = %e,
                    principal = principal_id,
                    action,
                    resource = resource_id,
                    "authorization request could not be evaluated — denying (fail-closed)"
                );
                AuthzDecision::Deny
            }
        }
    }

    /// The fallible core of [`Self::authorize`], separated so the error → deny
    /// mapping lives in exactly one place.
    ///
    /// Returns the Cedar decision plus an optional [`ApprovalContext`] when a
    /// matched permit policy is annotated with `@require_approval`.
    fn evaluate(
        &self,
        bundle: &CedarBundle,
        kind: PrincipalKind,
        principal_id: &str,
        action: &str,
        resource_id: &str,
        context: &Value,
    ) -> Result<(Decision, Option<ApprovalContext>), AuthzError> {
        let principal = make_uid(kind.cedar_type(), principal_id)?;
        let action_uid = make_uid(ACTION_TYPE, action)?;
        let resource = make_uid(RESOURCE_TYPE, resource_id)?;
        let cedar_context = build_context(context)?;

        let request = Request::new(
            principal,
            action_uid,
            resource,
            cedar_context,
            bundle.schema(),
        )
        .map_err(|e| AuthzError::RequestBuild(e.to_string()))?;

        let response =
            self.authorizer
                .is_authorized(&request, bundle.policies(), bundle.entities());
        let approval =
            extract_approval_context(&response, bundle, principal_id, action, resource_id);
        Ok((response.decision(), approval))
    }
}

/// If the Cedar response is `Allow` and any matched permit policy carries the
/// `@require_approval` annotation, build an [`ApprovalContext`].
///
/// Only `permit` policies that actually contributed to the allow decision (i.e.
/// appear in the response diagnostics) are considered. `forbid` policies and
/// policies that did not match are ignored. When multiple matched policies are
/// annotated, their reason values are concatenated with `"; "`.
fn extract_approval_context(
    response: &cedar_policy::Response,
    bundle: &CedarBundle,
    principal_id: &str,
    action: &str,
    resource_id: &str,
) -> Option<ApprovalContext> {
    if response.decision() != Decision::Allow {
        return None;
    }

    let mut requires_approval = false;
    let mut reasons: Vec<String> = Vec::new();
    for policy_id in response.diagnostics().reason() {
        let Some(policy) = bundle.policies().policies().find(|p| {
            <PolicyId as AsRef<str>>::as_ref(p.id()) == <PolicyId as AsRef<str>>::as_ref(policy_id)
        }) else {
            continue;
        };
        // Forbid policies annotated with @require_approval must NOT promote to
        // RequireApproval; they remain Deny (already filtered by Allow above,
        // but guard explicitly for clarity and defense in depth).
        if !matches!(policy.effect(), Effect::Permit) {
            continue;
        }
        for (key, value) in policy.annotations() {
            if key == "require_approval" {
                requires_approval = true;
                if !value.is_empty() {
                    reasons.push(value.to_string());
                }
                break;
            }
        }
    }

    if !requires_approval {
        return None;
    }

    let reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };

    Some(ApprovalContext::new(
        principal_id,
        action,
        resource_id,
        reason,
    ))
}

/// Construct a Cedar `EntityUid` of `Type::"id"` from an untrusted string id.
///
/// Cedar entity ids are arbitrary strings, so `EntityId::from_str` accepts any
/// value; the only fallible part is the (constant) type name.
fn make_uid(type_name: &str, id: &str) -> Result<EntityUid, AuthzError> {
    let etype = EntityTypeName::from_str(type_name)
        .map_err(|e| AuthzError::RequestBuild(format!("bad entity type `{type_name}`: {e}")))?;
    let eid = EntityId::from_str(id)
        .map_err(|e| AuthzError::RequestBuild(format!("bad entity id `{id}`: {e}")))?;
    Ok(EntityUid::from_type_name_and_id(etype, eid))
}

/// Map a JSON value into a Cedar request `Context`.
///
/// A JSON object becomes the context record. `Null` becomes an empty context.
/// Any other JSON shape (array, scalar) is rejected — a context must be a
/// record — which the caller treats as fail-closed. The schema is not applied
/// here; the full request (including this context) is validated against the
/// bundle schema in `Request::new`.
fn build_context(value: &Value) -> Result<Context, AuthzError> {
    match value {
        Value::Null => Ok(Context::empty()),
        Value::Object(_) => Context::from_json_value(value.clone(), None)
            .map_err(|e| AuthzError::RequestBuild(format!("invalid context record: {e}"))),
        _ => Err(AuthzError::RequestBuild(
            "context must be a JSON object".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(id: &str, text: &str) -> PolicyRecord {
        PolicyRecord {
            id: id.to_string(),
            policy_text: text.to_string(),
            schema_json: None,
            entities_json: None,
        }
    }

    #[test]
    fn allow_decision_when_permit_matches() {
        let engine =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        let decision = engine.authorize("alice", DEFAULT_ACTION, "route-1", &json!({}));
        assert_eq!(decision, AuthzDecision::Allow);
    }

    // ── Distinct principal types (Agent / Service vs User) ────────────────

    #[test]
    fn agent_scoped_policy_allows_agent_but_denies_user() {
        // A policy that names an Agent principal must NOT match a User with the
        // same id — the entity TYPE distinguishes them.
        let engine = AuthzEngine::from_records(&[record(
            "agent-only",
            r#"permit(principal == Agent::"bot-7", action, resource);"#,
        )])
        .expect("compiles");

        // Agent "bot-7" → allowed.
        assert_eq!(
            engine.authorize_as(PrincipalKind::Agent, "bot-7", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
        // User "bot-7" (same id, different type) → denied.
        assert_eq!(
            engine.authorize_as(PrincipalKind::User, "bot-7", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
        // The back-compat `authorize` (User) is likewise denied.
        assert_eq!(
            engine.authorize("bot-7", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn user_scoped_policy_denies_agent() {
        // The inverse: a User-scoped permit must not match an Agent principal.
        let engine = AuthzEngine::from_records(&[record(
            "user-only",
            r#"permit(principal == User::"alice", action, resource);"#,
        )])
        .expect("compiles");

        assert_eq!(
            engine.authorize_as(PrincipalKind::User, "alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
        assert_eq!(
            engine.authorize_as(PrincipalKind::Agent, "alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn service_principal_type_is_distinct() {
        let engine = AuthzEngine::from_records(&[record(
            "svc-only",
            r#"permit(principal == Service::"deploy-svc", action, resource);"#,
        )])
        .expect("compiles");
        assert_eq!(
            engine.authorize_as(PrincipalKind::Service, "deploy-svc", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
        assert_eq!(
            engine.authorize_as(PrincipalKind::Agent, "deploy-svc", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn principal_kind_maps_to_expected_cedar_type() {
        assert_eq!(PrincipalKind::User.cedar_type(), "User");
        assert_eq!(PrincipalKind::Agent.cedar_type(), "Agent");
        assert_eq!(PrincipalKind::Service.cedar_type(), "Service");
    }

    #[test]
    fn deny_decision_when_no_policy_permits() {
        // Empty policy set → Cedar default-deny.
        let engine = AuthzEngine::empty();
        let decision = engine.authorize("alice", DEFAULT_ACTION, "route-1", &json!({}));
        assert_eq!(decision, AuthzDecision::Deny);
    }

    #[test]
    fn deny_decision_when_forbid_overrides_permit() {
        let engine = AuthzEngine::from_records(&[
            record("permit", r#"permit(principal, action, resource);"#),
            record(
                "forbid",
                r#"forbid(principal, action, resource) when { principal == User::"blocked" };"#,
            ),
        ])
        .expect("compiles");
        assert_eq!(
            engine.authorize("blocked", DEFAULT_ACTION, "route-1", &json!({})),
            AuthzDecision::Deny,
            "forbid must override permit"
        );
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "route-1", &json!({})),
            AuthzDecision::Allow
        );
    }

    #[test]
    fn permit_conditioned_on_context_attribute() {
        let engine = AuthzEngine::from_records(&[record(
            "ctx",
            r#"permit(principal, action, resource) when { context.method == "GET" };"#,
        )])
        .expect("compiles");
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({"method": "GET"})),
            AuthzDecision::Allow
        );
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({"method": "POST"})),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn non_object_context_fails_closed() {
        let engine =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        // A JSON array is not a valid context record → deny, not allow.
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!([1, 2, 3])),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn reload_retains_last_good_on_bad_policy() {
        // Start with a permit; confirm allow.
        let engine =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );

        // A malformed reload must FAIL and RETAIN the last-good bundle.
        let err = engine
            .reload_from_records(&[record("broken", "not valid cedar {{{")])
            .unwrap_err();
        assert!(matches!(err, AuthzError::PolicyParse(_)));

        // The engine still authorizes with the previous (good) bundle.
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow,
            "last-good bundle must survive a failed reload"
        );
    }

    #[test]
    fn reload_swaps_in_new_good_bundle() {
        let engine = AuthzEngine::empty();
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
        engine
            .reload_from_records(&[record("p", r#"permit(principal, action, resource);"#)])
            .expect("good reload swaps");
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
    }

    // ── add-hitl-approval: RequireApproval via @require_approval annotation ─

    #[test]
    fn require_approval_when_permit_policy_is_annotated() {
        let engine = AuthzEngine::from_records(&[record(
            "approval",
            r#"@require_approval("Sensitive operation requires human review")
            permit(principal, action, resource);"#,
        )])
        .expect("compiles");
        let decision = engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({}));
        assert!(
            decision.is_require_approval(),
            "annotated permit policy must yield RequireApproval"
        );
        let ctx = decision.approval_context().expect("context present");
        assert_eq!(ctx.principal_id, "alice");
        assert_eq!(ctx.action, DEFAULT_ACTION);
        assert_eq!(ctx.resource_id, "r1");
        assert_eq!(
            ctx.reason.as_deref(),
            Some("Sensitive operation requires human review")
        );
        assert!(!ctx.is_expired());
    }

    #[test]
    fn require_approval_without_reason_omits_reason() {
        // Cedar annotation syntax requires parentheses; an empty string value
        // represents "reason omitted" and maps to `None`.
        let engine = AuthzEngine::from_records(&[record(
            "approval",
            r#"@require_approval("")
            permit(principal, action, resource);"#,
        )])
        .expect("compiles");
        let decision = engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({}));
        assert!(decision.is_require_approval());
        let ctx = decision.approval_context().unwrap();
        assert!(ctx.reason.is_none());
    }

    #[test]
    fn annotated_forbid_does_not_require_approval() {
        // A forbid policy annotated with @require_approval must still deny and
        // must NOT promote to RequireApproval.
        let engine = AuthzEngine::from_records(&[record(
            "blocked",
            r#"@require_approval("should be ignored")
                forbid(principal, action, resource);"#,
        )])
        .expect("compiles");
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn unannotated_permit_still_allows() {
        let engine =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        let decision = engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({}));
        assert_eq!(decision, AuthzDecision::Allow);
        assert!(!decision.is_require_approval());
    }

    #[test]
    fn require_approval_fails_closed_on_error() {
        // The engine itself cannot fail here, but the contract is that any
        // evaluation error maps to Deny, not RequireApproval.
        let engine = AuthzEngine::empty();
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );
    }

    // ── H2 / C1: lenient reload path (what a "policies" NOTIFY drives) ────────

    #[test]
    fn lenient_reload_skips_bad_row_keeps_good() {
        let engine = AuthzEngine::empty();
        engine.reload_from_records_lenient(&[
            record("good", r#"permit(principal, action, resource);"#),
            record("poisoned", "not cedar {{{"),
        ]);
        // The good row applied despite the poisoned sibling.
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
    }

    #[test]
    fn lenient_reload_all_bad_degrades_to_deny_not_open() {
        // Start allowing, then a fully-poisoned reload → default-deny, NOT open.
        let engine =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        engine.reload_from_records_lenient(&[record("bad", "garbage {{{")]);
        assert_eq!(
            engine.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny,
            "a fully poisoned reload must fail closed, never fail open"
        );
    }

    #[test]
    fn second_engine_reflects_change_after_reload() {
        // Simulates C1: two independent engine instances (as on two replicas).
        // A policy change reloaded into the "peer" is reflected in its decisions
        // — the mechanism a "policies" NOTIFY drives via reload_from_database.
        let primary =
            AuthzEngine::from_records(&[record("p", r#"permit(principal, action, resource);"#)])
                .expect("compiles");
        let peer = AuthzEngine::empty();

        // Peer starts default-deny (no policies yet).
        assert_eq!(
            peer.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Deny
        );

        // A "policies" NOTIFY arrives on the peer → it reloads the same policy
        // set the primary already has.
        peer.reload_from_records_lenient(&[record("p", r#"permit(principal, action, resource);"#)]);

        // Now the peer authorizes identically to the primary — no restart.
        assert_eq!(
            peer.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
        assert_eq!(
            primary.authorize("alice", DEFAULT_ACTION, "r1", &json!({})),
            AuthzDecision::Allow
        );
    }
}
