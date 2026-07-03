//! The shared authorization engine: a lock-free-on-read Cedar evaluator.

use std::str::FromStr;
use std::sync::Arc;

use arc_swap::ArcSwap;
use cedar_policy::{Authorizer, Context, Decision, EntityId, EntityTypeName, EntityUid, Request};
use serde_json::Value;
use tracing::{error, warn};

use super::bundle::{CedarBundle, PolicyRecord};
use super::error::AuthzError;

/// Cedar entity type used for the request principal.
const PRINCIPAL_TYPE: &str = "User";
/// Cedar entity type used for the request action.
const ACTION_TYPE: &str = "Action";
/// Cedar entity type used for the request resource (a route).
const RESOURCE_TYPE: &str = "Route";
/// Default generic action id when a hook does not specify one.
pub const DEFAULT_ACTION: &str = "invoke";

/// The outcome of an authorization check.
///
/// This is a deliberately tiny surface: the pipeline only needs allow-vs-deny.
/// Every error, ambiguity, or construction failure collapses to [`Self::Deny`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzDecision {
    /// The request is permitted.
    Allow,
    /// The request is denied (explicitly, or fail-closed on any error).
    Deny,
}

impl AuthzDecision {
    /// Is this an allow?
    pub fn is_allow(self) -> bool {
        matches!(self, AuthzDecision::Allow)
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
    pub fn authorize(
        &self,
        principal_id: &str,
        action: &str,
        resource_id: &str,
        context: &Value,
    ) -> AuthzDecision {
        let snapshot = self.bundle.load();
        match self.evaluate(&snapshot, principal_id, action, resource_id, context) {
            Ok(decision) => match decision {
                Decision::Allow => AuthzDecision::Allow,
                Decision::Deny => AuthzDecision::Deny,
            },
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
    fn evaluate(
        &self,
        bundle: &CedarBundle,
        principal_id: &str,
        action: &str,
        resource_id: &str,
        context: &Value,
    ) -> Result<Decision, AuthzError> {
        let principal = make_uid(PRINCIPAL_TYPE, principal_id)?;
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
        Ok(response.decision())
    }
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
