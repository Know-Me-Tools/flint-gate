//! Agent tool-scope **sugar** → Cedar compiler.
//!
//! Operators can express per-agent tool scoping ergonomically:
//!
//! ```yaml
//! agent_tool_policies:
//!   - agent: "ci-bot"
//!     allow: ["deploy", "run_tests"]
//!     deny:  ["delete_*"]
//! ```
//!
//! This module compiles each entry into the **same** Cedar the engine already
//! runs — `permit`/`forbid` statements on `Action::"call_tool"` for the
//! `Agent::"<agent>"` principal — so the sugar is a validated front-end, never a
//! second policy authority (federate/validate, never a second engine). The
//! emitted [`PolicyRecord`]s are passed through the existing write-time
//! [`validate_policy`](super::validate_policy) gate before load; a sugar block
//! that compiles to invalid Cedar is rejected (fail-closed — a bad policy never
//! loads).
//!
//! ## Deny wins
//!
//! Each `allow` tool becomes a `permit`; each `deny` tool becomes a `forbid`.
//! Cedar `forbid` **always overrides** `permit`, so `deny` beats `allow` even
//! when both name the same tool — enforced by Cedar's evaluation, not by us.
//!
//! ## Injection safety
//!
//! Agent ids and tool names are compiled into Cedar **source text**, so an
//! untrusted string could otherwise break out of its literal. We reject any id
//! that is not a conservative identifier up front ([`is_valid_agent`] /
//! [`is_valid_tool`]) rather than trying to escape it — the emitted text can then
//! only ever contain a known-safe token inside its quotes.

use super::bundle::PolicyRecord;
use super::error::AuthzError;
use crate::config::types::AgentToolPolicy;

/// Cedar action all tool-call policies are scoped to (mirrors
/// [`super::ACTION_CALL_TOOL`]).
const ACTION_CALL_TOOL: &str = "call_tool";

/// Reserved `PolicyRecord::id` prefix for compiled sugar policies. DB-stored
/// policy ids MUST NOT use this prefix — the admin/DB write path rejects any that
/// do, so a stored policy can never collide with (and silently suppress) a sugar
/// overlay policy when the two are merged into one `PolicySet`. The single source
/// of truth for the `agent_tool_sugar::<agent>::<index>` id scheme below.
pub const SUGAR_ID_PREFIX: &str = "agent_tool_sugar::";

/// Whether `c` is allowed in an agent id or an **exact** tool name. Conservative
/// on purpose: alphanumerics plus the separators real ids use. No quote,
/// backslash, or whitespace can appear, so the value is always a safe Cedar
/// string-literal body.
fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')
}

/// A valid agent id: non-empty, only [`is_id_char`] characters.
fn is_valid_agent(agent: &str) -> bool {
    !agent.is_empty() && agent.chars().all(is_id_char)
}

/// A valid tool token: non-empty, [`is_id_char`] characters plus `*` (the glob
/// wildcard). `*` selects the glob compilation path.
fn is_valid_tool(tool: &str) -> bool {
    !tool.is_empty() && tool.chars().all(|c| is_id_char(c) || c == '*')
}

/// Compile an `agent_tool_policies` set into validated Cedar [`PolicyRecord`]s.
///
/// One record per entry, its `policy_text` holding that agent's `permit`/`forbid`
/// statements. An entry with an empty/illegal agent id or tool name is rejected
/// with [`AuthzError::SugarCompile`] (fail-closed). An entry with neither `allow`
/// nor `deny` compiles to an empty record and is skipped (nothing to enforce).
///
/// The returned records are NOT yet validated against a schema — callers pass
/// them through [`validate_policy`](super::validate_policy) (see
/// [`compile_and_validate`]).
pub fn compile_agent_tool_policies(
    policies: &[AgentToolPolicy],
) -> Result<Vec<PolicyRecord>, AuthzError> {
    let mut records = Vec::new();
    for (i, entry) in policies.iter().enumerate() {
        if !is_valid_agent(&entry.agent) {
            return Err(AuthzError::SugarCompile(format!(
                "entry {i}: agent id {:?} is empty or contains illegal characters \
                 (allowed: alphanumerics and _-.:)",
                entry.agent
            )));
        }
        let mut statements = Vec::new();
        for tool in &entry.allow {
            statements.push(compile_statement("permit", &entry.agent, tool, i)?);
        }
        for tool in &entry.deny {
            statements.push(compile_statement("forbid", &entry.agent, tool, i)?);
        }
        if statements.is_empty() {
            continue; // nothing to enforce for this agent
        }
        records.push(PolicyRecord {
            id: format!("{SUGAR_ID_PREFIX}{}::{i}", entry.agent),
            policy_text: statements.join("\n"),
            schema_json: None,
            entities_json: None,
        });
    }
    Ok(records)
}

/// Compile one `permit`/`forbid` statement for `agent` scoped to `tool`.
///
/// - exact tool  → `... resource == Route::"<tool>";`
/// - glob (`*`)  → `... resource) when { context.tool_name like "<glob>" };`
///   (Cedar `like` uses `*` as its wildcard, matching the sugar glob directly.)
fn compile_statement(
    effect: &str,
    agent: &str,
    tool: &str,
    entry_index: usize,
) -> Result<String, AuthzError> {
    if !is_valid_tool(tool) {
        return Err(AuthzError::SugarCompile(format!(
            "entry {entry_index}: tool {tool:?} is empty or contains illegal \
             characters (allowed: alphanumerics, _-.:, and * for globs)"
        )));
    }
    let head = format!(
        "{effect}(principal == Agent::\"{agent}\", action == Action::\"{ACTION_CALL_TOOL}\", resource"
    );
    if tool.contains('*') {
        // Glob: match the tool name in context. `resource` stays unconstrained in
        // the scope; the `like` clause carries the wildcard.
        Ok(format!(
            "{head}) when {{ context.tool_name like \"{tool}\" }};"
        ))
    } else {
        // Exact tool → resource-scoped.
        Ok(format!("{head} == Route::\"{tool}\");"))
    }
}

/// Compile the sugar AND run every emitted record through the write-time
/// validator, so an entry that produces invalid Cedar is rejected at load
/// (fail-closed). Returns the validated records ready to merge into the engine.
pub fn compile_and_validate(
    policies: &[AgentToolPolicy],
) -> Result<Vec<PolicyRecord>, AuthzError> {
    let records = compile_agent_tool_policies(policies)?;
    for record in &records {
        super::validate_policy(record)?;
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::{authorize_tool_call, AuthzEngine, PrincipalKind};
    use serde_json::json;

    fn policy(agent: &str, allow: &[&str], deny: &[&str]) -> AgentToolPolicy {
        AgentToolPolicy {
            agent: agent.to_string(),
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Build an engine from the compiled sugar (as the loader would).
    fn engine_from(policies: &[AgentToolPolicy]) -> AuthzEngine {
        let records = compile_and_validate(policies).expect("valid sugar compiles");
        AuthzEngine::from_records(&records).expect("records build an engine")
    }

    /// Authorize an agent tool call through the REAL runtime entry point, so the
    /// Cedar `context.tool_name` (which glob policies match on) is populated
    /// exactly as it is on the streaming hot path.
    fn agent_can_call(engine: &AuthzEngine, agent: &str, tool: &str) -> bool {
        authorize_tool_call(engine, PrincipalKind::Agent, agent, tool, &json!({}), "route-1")
            .is_allow()
    }

    #[test]
    fn allow_only_compiles_and_authorizes() {
        let e = engine_from(&[policy("ci-bot", &["deploy", "run_tests"], &[])]);
        assert!(agent_can_call(&e, "ci-bot", "deploy"));
        assert!(agent_can_call(&e, "ci-bot", "run_tests"));
        // A tool not in the allow list is denied (default-deny).
        assert!(!agent_can_call(&e, "ci-bot", "delete_all"));
    }

    #[test]
    fn deny_overrides_allow_forbid_wins() {
        // Same tool in both allow and deny → forbid wins (Cedar semantics).
        let e = engine_from(&[policy("ci-bot", &["deploy"], &["deploy"])]);
        assert!(!agent_can_call(&e, "ci-bot", "deploy"));
    }

    #[test]
    fn glob_deny_blocks_matching_tools() {
        // allow a broad set, deny the destructive glob.
        let e = engine_from(&[policy("ci-bot", &["deploy", "delete_widget"], &["delete_*"])]);
        // The non-matching allowed tool still works.
        assert!(agent_can_call(&e, "ci-bot", "deploy"));
        // The glob-denied tool is blocked even though it was explicitly allowed.
        assert!(!agent_can_call(&e, "ci-bot", "delete_widget"));
    }

    #[test]
    fn glob_allow_permits_matching_tools() {
        let e = engine_from(&[policy("reader", &["read_*"], &[])]);
        assert!(agent_can_call(&e, "reader", "read_file"));
        assert!(!agent_can_call(&e, "reader", "write_file"));
    }

    #[test]
    fn policy_is_scoped_to_its_agent() {
        // A different agent is not granted another agent's allows.
        let e = engine_from(&[policy("ci-bot", &["deploy"], &[])]);
        assert!(!agent_can_call(&e, "other-bot", "deploy"));
    }

    #[test]
    fn empty_agent_id_is_rejected() {
        let err = compile_and_validate(&[policy("", &["deploy"], &[])]).unwrap_err();
        assert!(matches!(err, AuthzError::SugarCompile(_)));
    }

    #[test]
    fn illegal_agent_id_is_rejected_injection_safe() {
        // A quote in the agent id would break out of the Cedar literal — reject it.
        let err =
            compile_and_validate(&[policy("ci\"bot", &["deploy"], &[])]).unwrap_err();
        assert!(matches!(err, AuthzError::SugarCompile(_)));
    }

    #[test]
    fn illegal_tool_name_is_rejected() {
        let err = compile_and_validate(&[policy("ci-bot", &["de ploy"], &[])]).unwrap_err();
        assert!(matches!(err, AuthzError::SugarCompile(_)));
    }

    #[test]
    fn entry_with_no_rules_compiles_to_nothing() {
        let records = compile_agent_tool_policies(&[policy("idle", &[], &[])]).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn compile_and_validate_gate_runs_the_validator() {
        // A valid block passes the write-time validator (the same gate the admin
        // CRUD path uses), producing loadable records.
        let records =
            compile_and_validate(&[policy("ci-bot", &["deploy"], &["delete_*"])]).unwrap();
        assert_eq!(records.len(), 1);
        // And every emitted record independently passes validate_policy.
        for r in &records {
            crate::authz::validate_policy(r).expect("emitted record is valid Cedar");
        }
    }

    #[test]
    fn validator_rejects_cedar_invalid_sugar_shaped_record() {
        // Defense-in-depth: even if the compiler ever emitted malformed Cedar, the
        // validator gate that compile_and_validate runs would reject it (fail-
        // closed). Feed the gate a deliberately-broken sugar-shaped record.
        let bad = PolicyRecord {
            id: "agent_tool_sugar::x::0".to_string(),
            policy_text: "permit(principal == Agent::\"x\", action ==".to_string(), // truncated
            schema_json: None,
            entities_json: None,
        };
        assert!(crate::authz::validate_policy(&bad).is_err());
    }
}
