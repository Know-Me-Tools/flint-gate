//! Write-time policy validation.
//!
//! The admin CRUD path calls [`validate_policy`] BEFORE persisting a policy so
//! that malformed or schema-incoherent policy text never reaches the database
//! or the hot path. This is the single gate: if it returns `Ok`, the policy is
//! parseable and (when a schema is supplied) type-checks against that schema.

use std::str::FromStr;

use cedar_policy::{
    ActionConstraint, Effect, Entities, PolicySet, PrincipalConstraint, ResourceConstraint, Schema,
    ValidationMode, Validator,
};

use super::bundle::PolicyRecord;
use super::error::AuthzError;

/// Warning message emitted when a policy grants broad, unconditional access.
pub const ALLOW_ALL_WARNING: &str = "policy grants broad/unconditional access";

/// Validate a candidate policy before it is stored.
///
/// 1. The policy text MUST parse into a [`PolicySet`].
/// 2. If `schema_json` is present, it MUST parse into a [`Schema`], and the
///    policy set MUST pass the Cedar [`Validator`] in strict mode against it.
///
/// Any failure returns a typed [`AuthzError`] whose message is safe to surface
/// to the admin caller (it is policy/schema text they authored, not internal
/// state).
pub fn validate_policy(record: &PolicyRecord) -> Result<(), AuthzError> {
    let policy_set = PolicySet::from_str(&record.policy_text)
        .map_err(|e| AuthzError::PolicyParse(e.to_string()))?;

    // Parse the schema up front (if any) so it can be applied to BOTH the policy
    // validator and the entities parse — the loader parses entities against the
    // schema, so write-time validation must do the same to be a true superset.
    let schema = match &record.schema_json {
        None => None,
        Some(serde_json::Value::String(src)) => {
            Some(Schema::from_str(src).map_err(|e| AuthzError::SchemaParse(e.to_string()))?)
        }
        Some(other) => Some(
            Schema::from_json_value(other.clone())
                .map_err(|e| AuthzError::SchemaParse(e.to_string()))?,
        ),
    };

    // Validate entities EXACTLY as the loader (`CedarBundle::from_records`) does,
    // so a bad entities blob is rejected at the 400 gate rather than passing the
    // write and then silently failing the reload build (H1).
    if let Some(entities_value) = &record.entities_json {
        Entities::from_json_value(entities_value.clone(), schema.as_ref())
            .map_err(|e| AuthzError::EntitiesParse(e.to_string()))?;
    }

    // No schema → a parseable policy (and, above, a parseable entities blob) is
    // accepted. Cedar still enforces policy syntax at parse time.
    let Some(schema) = schema else {
        return Ok(());
    };

    let validator = Validator::new(schema);
    let result = validator.validate(&policy_set, ValidationMode::Strict);
    if result.validation_passed() {
        Ok(())
    } else {
        let messages: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
        Err(AuthzError::Validation(messages.join("; ")))
    }
}

/// Non-blocking guardrail: collect advisory warnings about a policy's breadth.
///
/// Currently detects an "allow-all" `permit` — one with an empty `when`/`unless`
/// and an unconstrained principal, action, AND resource — which grants
/// unconditional access. Broad permits are sometimes legitimate, so this NEVER
/// blocks; the admin API surfaces the warnings in the 200 response so an
/// operator can notice an accidental blanket allow.
///
/// Returns an empty vec when the policy text does not parse (validation, which
/// runs first, owns rejection of unparseable policy).
pub fn policy_warnings(record: &PolicyRecord) -> Vec<String> {
    let Ok(policy_set) = PolicySet::from_str(&record.policy_text) else {
        return Vec::new();
    };
    if policy_set.policies().any(is_allow_all) {
        vec![ALLOW_ALL_WARNING.to_string()]
    } else {
        Vec::new()
    }
}

/// Is this a `permit` that grants unconditional, unconstrained access?
fn is_allow_all(policy: &cedar_policy::Policy) -> bool {
    matches!(policy.effect(), Effect::Permit)
        && !policy.has_non_scope_constraint()
        && matches!(policy.principal_constraint(), PrincipalConstraint::Any)
        && matches!(policy.action_constraint(), ActionConstraint::Any)
        && matches!(policy.resource_constraint(), ResourceConstraint::Any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(text: &str, schema: Option<serde_json::Value>) -> PolicyRecord {
        PolicyRecord {
            id: "p".to_string(),
            policy_text: text.to_string(),
            schema_json: schema,
            entities_json: None,
        }
    }

    #[test]
    fn accepts_parseable_policy_without_schema() {
        let rec = record(r#"permit(principal, action, resource);"#, None);
        assert!(validate_policy(&rec).is_ok());
    }

    #[test]
    fn rejects_unparseable_policy() {
        let rec = record("not cedar {{{", None);
        let err = validate_policy(&rec).unwrap_err();
        assert!(matches!(err, AuthzError::PolicyParse(_)));
    }

    #[test]
    fn accepts_policy_that_typechecks_against_schema() {
        let schema = serde_json::Value::String(
            "entity User; entity Route; action \"invoke\" appliesTo { principal: [User], resource: [Route] };".to_string(),
        );
        let rec = record(
            r#"permit(principal, action == Action::"invoke", resource);"#,
            Some(schema),
        );
        assert!(validate_policy(&rec).is_ok(), "should type-check");
    }

    #[test]
    fn rejects_policy_referencing_unknown_action_under_schema() {
        let schema = serde_json::Value::String(
            "entity User; entity Route; action \"invoke\" appliesTo { principal: [User], resource: [Route] };".to_string(),
        );
        // References an action the schema does not define → strict validation fails.
        let rec = record(
            r#"permit(principal, action == Action::"delete_everything", resource);"#,
            Some(schema),
        );
        let err = validate_policy(&rec).unwrap_err();
        assert!(
            matches!(err, AuthzError::Validation(_)),
            "unknown action must fail schema validation, got {err:?}"
        );
    }

    #[test]
    fn rejects_malformed_schema() {
        let rec = record(
            r#"permit(principal, action, resource);"#,
            Some(serde_json::Value::String("entity {{{ broken".to_string())),
        );
        let err = validate_policy(&rec).unwrap_err();
        assert!(matches!(err, AuthzError::SchemaParse(_)));
    }

    // ── H1: entities_json is validated at write time ─────────────────────────

    #[test]
    fn rejects_bad_entities_json_at_write_time() {
        // A structurally invalid entities blob must be rejected by the 400 gate
        // so it can never be stored enabled and then fail the reload silently.
        let mut rec = record(r#"permit(principal, action, resource);"#, None);
        rec.entities_json = Some(serde_json::json!({"not": "an entities array"}));
        let err = validate_policy(&rec).unwrap_err();
        assert!(
            matches!(err, AuthzError::EntitiesParse(_)),
            "bad entities must fail write-time validation, got {err:?}"
        );
    }

    #[test]
    fn accepts_valid_empty_entities_json() {
        let mut rec = record(r#"permit(principal, action, resource);"#, None);
        rec.entities_json = Some(serde_json::json!([]));
        assert!(validate_policy(&rec).is_ok());
    }

    // ── M1: allow-all breadth detection (non-blocking) ───────────────────────

    #[test]
    fn detects_allow_all_permit() {
        let rec = record(r#"permit(principal, action, resource);"#, None);
        let warnings = policy_warnings(&rec);
        assert_eq!(warnings, vec![ALLOW_ALL_WARNING.to_string()]);
    }

    #[test]
    fn no_warning_for_constrained_permit() {
        let rec = record(
            r#"permit(principal == User::"alice", action, resource);"#,
            None,
        );
        assert!(
            policy_warnings(&rec).is_empty(),
            "a principal-constrained permit is not allow-all"
        );
    }

    #[test]
    fn no_warning_for_conditioned_permit() {
        let rec = record(
            r#"permit(principal, action, resource) when { context.ok == true };"#,
            None,
        );
        assert!(
            policy_warnings(&rec).is_empty(),
            "a `when`-conditioned permit is not unconditional"
        );
    }

    #[test]
    fn no_warning_for_forbid_all() {
        // A blanket forbid is not an over-permissive grant.
        let rec = record(r#"forbid(principal, action, resource);"#, None);
        assert!(policy_warnings(&rec).is_empty());
    }

    #[test]
    fn allow_all_detected_among_multiple_policies() {
        let rec = record(
            r#"permit(principal == User::"a", action, resource);
               permit(principal, action, resource);"#,
            None,
        );
        assert_eq!(policy_warnings(&rec), vec![ALLOW_ALL_WARNING.to_string()]);
    }
}
