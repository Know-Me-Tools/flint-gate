//! Gateway Cedar entity schema — the canonical type model for all policies stored
//! in flint-gate.
//!
//! ## Entity model
//!
//! | Kind       | Cedar type | Description                                  |
//! |------------|-----------|----------------------------------------------|
//! | Principal  | `User`    | Human user (JWT sub)                         |
//! | Principal  | `Agent`   | Delegated agent identity (NHI)               |
//! | Principal  | `Service` | Service / workload identity (client_creds)   |
//! | Resource   | `Route`   | A proxy route (the Cedar resource)           |
//! | Action     | `call_tool` | The only valid gateway action               |
//!
//! ## Annotation semantics
//!
//! `@require_approval("reason")` on a `permit` policy causes the gateway to
//! pause the tool call and request human approval before forwarding. Any
//! annotation key OTHER THAN `require_approval` on a gateway policy is rejected
//! at write time: it is either a typo of a known annotation or an unsupported
//! extension.

/// The gateway Cedar schema in human-readable Cedar syntax.
///
/// Policies written through the admin API are validated against this schema so
/// that entity-type typos, undefined actions, and misused annotations are caught
/// before they are persisted.
pub const GATEWAY_CEDAR_SCHEMA: &str = r#"
entity User;
entity Agent;
entity Service;
entity Route;

action "call_tool" appliesTo {
    principal: [User, Agent, Service],
    resource:  [Route]
};
"#;

/// Known annotation keys for gateway policies.
///
/// Policies may use `@require_approval("reason")` to signal that a matching
/// tool call requires human approval before forwarding. No other annotation keys
/// are defined; unrecognised keys are rejected at write time as likely typos.
pub const KNOWN_ANNOTATIONS: &[&str] = &["require_approval"];

/// Validate all annotations on a Cedar policy set against the gateway's known
/// annotation vocabulary. Returns an error string for the first unknown key
/// found, or `Ok(())` if all annotations are recognised.
///
/// Cedar itself treats annotations as free-form metadata and never rejects
/// unknown keys — this function fills that gap for gateway-specific semantics.
pub fn validate_annotations(policy_text: &str) -> Result<(), String> {
    use std::str::FromStr;
    let policy_set = match cedar_policy::PolicySet::from_str(policy_text) {
        Ok(ps) => ps,
        Err(_) => return Ok(()), // syntax errors are caught separately by validate_policy
    };
    for policy in policy_set.policies() {
        for (key, _value) in policy.annotations() {
            let key_str = key.as_ref();
            if !KNOWN_ANNOTATIONS.contains(&key_str) {
                let policy_id: &str = policy.id().as_ref();
                return Err(format!(
                    "unknown annotation @{key_str} on policy {policy_id:?} — \
                     did you mean @require_approval? \
                     Supported annotations: {}",
                    KNOWN_ANNOTATIONS.join(", ")
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cedar_policy::Schema;
    use std::str::FromStr;

    #[test]
    fn gateway_schema_parses() {
        Schema::from_str(GATEWAY_CEDAR_SCHEMA)
            .expect("GATEWAY_CEDAR_SCHEMA must be valid Cedar human syntax");
    }

    #[test]
    fn known_require_approval_annotation_is_accepted() {
        let policy = r#"@require_approval("sensitive operation")
permit(principal, action, resource);"#;
        assert!(validate_annotations(policy).is_ok());
    }

    #[test]
    fn typo_annotation_require_apporval_is_rejected() {
        let policy = r#"@require_apporval("sensitive operation")
permit(principal, action, resource);"#;
        let err = validate_annotations(policy).unwrap_err();
        assert!(
            err.contains("require_apporval"),
            "error must name the unknown annotation: {err}"
        );
        assert!(
            err.contains("require_approval"),
            "error must suggest the correct annotation: {err}"
        );
    }

    #[test]
    fn policy_with_no_annotations_is_accepted() {
        let policy = r#"permit(principal, action, resource);"#;
        assert!(validate_annotations(policy).is_ok());
    }

    #[test]
    fn completely_unknown_annotation_is_rejected() {
        let policy = r#"@my_custom_thing("value")
permit(principal, action, resource);"#;
        let err = validate_annotations(policy).unwrap_err();
        assert!(err.contains("my_custom_thing"));
    }
}
