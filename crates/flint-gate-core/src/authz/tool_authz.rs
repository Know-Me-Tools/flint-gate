//! Per-tool-call authorization: map an MCP / AG-UI tool call onto a Cedar
//! request and evaluate it against the shared [`AuthzEngine`].
//!
//! This is authorization on the hot streaming path. Every ambiguity — a
//! missing tool name, a malformed event, an un-mappable context — resolves to
//! [`AuthzDecision::Deny`] (fail-closed). The engine itself is already
//! fail-closed; this layer only decides *what* to ask it and treats any
//! inability to ask as a denial.
//!
//! ## Cedar request shape
//!
//! A tool call is modeled generically, reusing the existing engine surface:
//! - principal → `User::"<principal_id>"` (the authenticated identity)
//! - action    → `Action::"call_tool"` (generic; the tool identity lives in
//!   the resource + context, not in distinct action ids)
//! - resource  → `Route::"<tool_name>"` (the tool being invoked)
//! - context   → `{ tool_name, arguments, route_id }`
//!
//! `list_tools` filtering reuses the same `call_tool` action so a single policy
//! ("may this principal call this tool?") governs both the live invocation and
//! its visibility in a listing — the client never sees a tool it could not call.

use std::sync::Arc;

use serde_json::{json, Value};

use super::engine::{AuthzDecision, AuthzEngine};

/// Generic Cedar action id for a tool invocation. The same action gates both a
/// live `call_tool` in the stream and a tool's visibility in `list_tools`.
pub const ACTION_CALL_TOOL: &str = "call_tool";

/// Best-effort sink for per-tool-call audit records.
///
/// Holds a shared `Arc<Database>` and the request id so a per-tool DENY can be
/// written to the authz audit trail without threading a `Database` through every
/// stream processor. Cheap to clone (an `Arc` plus a small owned string).
#[derive(Clone)]
pub struct ToolAuditSink {
    db: Arc<crate::db::Database>,
    request_id: String,
}

impl ToolAuditSink {
    /// Construct a sink from a shared database handle and the request id.
    pub fn new(db: Arc<crate::db::Database>, request_id: impl Into<String>) -> Self {
        Self {
            db,
            request_id: request_id.into(),
        }
    }
}

impl std::fmt::Debug for ToolAuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolAuditSink")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

/// The per-request authorization handle threaded into stream processors.
///
/// Cheap to clone: the engine is shared via `Arc` (lock-free on read), and the
/// two ids are small owned strings captured once per request. Presence of this
/// context is what *enables* per-tool authorization for a stream — routes
/// without it are entirely unaffected (backward-compatible).
#[derive(Clone)]
pub struct ToolAuthzContext {
    /// Shared Cedar engine (ArcSwap-backed; lock-free snapshot on read).
    pub engine: Arc<AuthzEngine>,
    /// Authenticated principal id → Cedar `User::"<principal_id>"`.
    pub principal_id: String,
    /// Route id carried into the Cedar context for route-scoped policies.
    pub route_id: String,
    /// Optional best-effort audit sink. When present, a per-tool DENY is written
    /// to the authz audit trail (non-blocking); `None` disables per-tool audit
    /// (e.g. in tests or when no DB is configured).
    pub audit: Option<ToolAuditSink>,
}

impl std::fmt::Debug for ToolAuthzContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolAuthzContext")
            .field("principal_id", &self.principal_id)
            .field("route_id", &self.route_id)
            .field("audit", &self.audit)
            .finish_non_exhaustive()
    }
}

impl ToolAuthzContext {
    /// Authorize a tool call by name + arguments against this context's engine.
    ///
    /// Convenience wrapper over [`authorize_tool_call`] that supplies the
    /// engine, principal, and route id from `self`. When a [`ToolAuditSink`] is
    /// present, a DENY is recorded to the authz audit trail best-effort and
    /// non-blocking (per-tool denials are the security-relevant events; allows
    /// are intentionally NOT recorded here to keep the streaming hot path cheap
    /// and avoid one audit row per streamed tool call).
    pub fn authorize(&self, tool_name: &str, arguments: &Value) -> AuthzDecision {
        let decision = authorize_tool_call(
            &self.engine,
            &self.principal_id,
            tool_name,
            arguments,
            &self.route_id,
        );
        if !decision.is_allow() {
            self.record_tool_deny(tool_name);
        }
        decision
    }

    /// Record a per-tool DENY on the audit trail, off the hot path.
    ///
    /// No-op when no [`ToolAuditSink`] is configured. Otherwise clones the shared
    /// `Arc<Database>` and `tokio::spawn`s the insert so a slow or failing write
    /// never blocks the stream; on error we `warn!` and move on. The tool name
    /// is the Cedar `resource`; the arguments are deliberately NOT persisted (a
    /// tool-call payload may carry sensitive parameters).
    fn record_tool_deny(&self, tool_name: &str) {
        let Some(sink) = &self.audit else {
            return;
        };
        let db = sink.db.clone();
        let record = crate::db::AuthzAuditRecord {
            request_id: Some(sink.request_id.clone()),
            principal: self.principal_id.clone(),
            action: ACTION_CALL_TOOL.to_string(),
            resource: tool_name.to_string(),
            decision: crate::db::AuthzAuditDecision::Deny,
            reason: Some("per-tool call denied".to_string()),
            context: Some(json!({
                "tool_name": tool_name,
                "route_id": self.route_id,
            })),
        };
        tokio::spawn(async move {
            if let Err(e) = db.log_authz_decision(&record).await {
                tracing::warn!(error = %e, "per-tool authz audit write failed (best-effort, ignored)");
            }
        });
    }
}

/// Map a single tool call onto a Cedar request and evaluate it. Fail-closed.
///
/// An empty `tool_name` is rejected up front (a nameless tool cannot be
/// authorized, so it is denied) — this keeps a malformed AG-UI `TOOL_CALL_START`
/// with no name from ever reaching the engine as an allow. All other
/// error/ambiguity handling lives in [`AuthzEngine::authorize`], which already
/// collapses every failure to [`AuthzDecision::Deny`].
pub fn authorize_tool_call(
    engine: &AuthzEngine,
    principal_id: &str,
    tool_name: &str,
    arguments: &Value,
    route_id: &str,
) -> AuthzDecision {
    if tool_name.trim().is_empty() {
        // A tool call with no name is unauthorizable — deny (fail-closed).
        return AuthzDecision::Deny;
    }
    let context = build_tool_context(tool_name, arguments, route_id);
    engine.authorize(principal_id, ACTION_CALL_TOOL, tool_name, &context)
}

/// Build the Cedar request `context` record for a tool call.
///
/// Kept as a plain JSON object so [`AuthzEngine::authorize`] maps it into a
/// Cedar context. `arguments` is passed through verbatim so policies can branch
/// on parameters (e.g. `context.arguments.path like "/etc/*"`); a non-object
/// `arguments` value is coerced to an empty object because the surrounding
/// context must remain a valid record — the tool name and route are still
/// present so a by-name policy still evaluates correctly.
fn build_tool_context(tool_name: &str, arguments: &Value, route_id: &str) -> Value {
    let args = if arguments.is_object() {
        arguments.clone()
    } else {
        json!({})
    };
    json!({
        "tool_name": tool_name,
        "arguments": args,
        "route_id": route_id,
    })
}

/// Filter denied tools out of an MCP `tools/list` JSON-RPC response, in place.
///
/// MCP tool listings are JSON-RPC 2.0 responses shaped as
/// `{"jsonrpc":"2.0","id":…,"result":{"tools":[ {"name":…}, … ]}}`. Each tool
/// in `result.tools` is authorized via [`authorize_tool_call`] (empty args,
/// same `call_tool` action as a live invocation) and removed when it evaluates
/// `Deny`. The client then never sees a tool it could not call — the
/// agentgateway pattern.
///
/// **Fail-closed recognition (H2).** A value is recognized as a tools/list
/// response when it carries a `result` object that has a `tools` member. Once
/// recognized:
/// - `result.tools` is an array → filter it per-tool (missing/blank name → drop).
/// - `result.tools` is present but NOT an array → the payload is malformed; we
///   replace it with an EMPTY array rather than forward an unfilterable listing
///   (never leak tools we could not evaluate).
///
/// Also handles a JSON-RPC **batch** (a top-level array of responses): each
/// element is inspected and filtered independently; the batch counts as a
/// listing if ANY element is one.
///
/// Returns `true` if the value was recognized (and filtered) as a tools/list
/// response, `false` only when the message is definitively NOT one (a request, a
/// bare error response, or unrelated JSON) — those are forwarded untouched.
pub fn filter_list_tools_response(
    value: &mut Value,
    engine: &AuthzEngine,
    principal_id: &str,
    route_id: &str,
) -> bool {
    match value {
        // JSON-RPC batch: filter every element; it's a listing if any element is.
        Value::Array(items) => {
            let mut any = false;
            for item in items.iter_mut() {
                if filter_list_tools_response(item, engine, principal_id, route_id) {
                    any = true;
                }
            }
            any
        }
        Value::Object(_) => {
            // Definitively not a tools/list response unless `result.tools` exists.
            let Some(result) = value.get_mut("result") else {
                return false;
            };
            let Some(tools_val) = result.get_mut("tools") else {
                return false;
            };
            match tools_val.as_array_mut() {
                Some(tools) => {
                    let empty_args = json!({});
                    tools.retain(|tool| {
                        let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
                        let allow =
                            authorize_tool_call(engine, principal_id, name, &empty_args, route_id)
                                .is_allow();
                        if !allow {
                            tracing::info!(tool = %name, "tool filtered from list_tools (deny)");
                        }
                        allow
                    });
                }
                None => {
                    // Recognized as a tools/list response, but the tools payload
                    // is not an array — fail-closed: strip to empty, don't leak.
                    tracing::warn!(
                        "tools/list response with non-array `tools` — stripping (fail-closed)"
                    );
                    *tools_val = Value::Array(Vec::new());
                }
            }
            true
        }
        _ => false,
    }
}

/// Try to filter an MCP `tools/list` response carried in a raw JSON byte body.
///
/// Parses `body` as JSON; if it is recognized as a `tools/list` response (single
/// or batch), filters denied tools (fail-closed on malformed tools payloads) and
/// returns the re-serialized bytes. Returns `None` when the body is not JSON or
/// is definitively not a tool listing, so the caller forwards the original bytes
/// unchanged (zero-cost for non-listing responses).
pub fn filter_list_tools_body(
    body: &[u8],
    engine: &AuthzEngine,
    principal_id: &str,
    route_id: &str,
) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    if !filter_list_tools_response(&mut value, engine, principal_id, route_id) {
        return None;
    }
    serde_json::to_vec(&value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::PolicyRecord;

    fn record(id: &str, text: &str) -> PolicyRecord {
        PolicyRecord {
            id: id.to_string(),
            policy_text: text.to_string(),
            schema_json: None,
            entities_json: None,
        }
    }

    fn engine_permit_all() -> AuthzEngine {
        AuthzEngine::from_records(&[record("p", "permit(principal, action, resource);")])
            .expect("compiles")
    }

    #[test]
    fn allowed_tool_call_returns_allow() {
        let engine = engine_permit_all();
        let decision = authorize_tool_call(&engine, "alice", "read_file", &json!({}), "route-1");
        assert_eq!(decision, AuthzDecision::Allow);
    }

    #[test]
    fn denied_when_no_policy_permits() {
        let engine = AuthzEngine::empty();
        let decision = authorize_tool_call(&engine, "alice", "read_file", &json!({}), "route-1");
        assert_eq!(decision, AuthzDecision::Deny);
    }

    #[test]
    fn empty_tool_name_denies_without_touching_engine() {
        // Even a permit-all engine must deny a nameless tool call.
        let engine = engine_permit_all();
        assert_eq!(
            authorize_tool_call(&engine, "alice", "", &json!({}), "route-1"),
            AuthzDecision::Deny
        );
        assert_eq!(
            authorize_tool_call(&engine, "alice", "   ", &json!({}), "route-1"),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn resource_is_the_tool_name() {
        // A policy scoped to a specific tool resource permits only that tool.
        let engine = AuthzEngine::from_records(&[record(
            "scoped",
            r#"permit(principal, action, resource == Route::"safe_tool");"#,
        )])
        .expect("compiles");
        assert_eq!(
            authorize_tool_call(&engine, "alice", "safe_tool", &json!({}), "r1"),
            AuthzDecision::Allow
        );
        assert_eq!(
            authorize_tool_call(&engine, "alice", "danger_tool", &json!({}), "r1"),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn policy_can_branch_on_tool_name_in_context() {
        let engine = AuthzEngine::from_records(&[record(
            "byname",
            r#"permit(principal, action, resource) when { context.tool_name == "allowed" };"#,
        )])
        .expect("compiles");
        assert_eq!(
            authorize_tool_call(&engine, "alice", "allowed", &json!({}), "r1"),
            AuthzDecision::Allow
        );
        assert_eq!(
            authorize_tool_call(&engine, "alice", "other", &json!({}), "r1"),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn policy_can_branch_on_arguments() {
        let engine = AuthzEngine::from_records(&[record(
            "byarg",
            r#"permit(principal, action, resource) when { context.arguments.safe == true };"#,
        )])
        .expect("compiles");
        assert_eq!(
            authorize_tool_call(&engine, "alice", "t", &json!({"safe": true}), "r1"),
            AuthzDecision::Allow
        );
        assert_eq!(
            authorize_tool_call(&engine, "alice", "t", &json!({"safe": false}), "r1"),
            AuthzDecision::Deny
        );
    }

    #[test]
    fn non_object_arguments_coerced_to_empty_record_still_evaluates() {
        // A by-name permit must still hold when arguments arrive as a non-object
        // (e.g. a partial/garbage args payload) — we coerce to {} rather than
        // fail the whole context to a non-record (which would deny by-name too).
        let engine = AuthzEngine::from_records(&[record(
            "byname",
            r#"permit(principal, action, resource) when { context.tool_name == "t" };"#,
        )])
        .expect("compiles");
        assert_eq!(
            authorize_tool_call(&engine, "alice", "t", &json!("not-an-object"), "r1"),
            AuthzDecision::Allow
        );
        assert_eq!(
            authorize_tool_call(&engine, "alice", "t", &Value::Null, "r1"),
            AuthzDecision::Allow
        );
    }

    #[test]
    fn context_authorize_helper_matches_free_function() {
        let ctx = ToolAuthzContext {
            engine: Arc::new(engine_permit_all()),
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        };
        assert_eq!(ctx.authorize("read_file", &json!({})), AuthzDecision::Allow);
    }

    // ── list_tools filtering ────────────────────────────────────────────────

    fn list_tools_response() -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "tools": [
                    {"name": "safe_tool", "description": "ok"},
                    {"name": "danger_tool", "description": "blocked"},
                    {"name": "other_tool", "description": "also blocked"}
                ]
            }
        })
    }

    #[test]
    fn filter_removes_denied_tools_keeps_allowed() {
        // Permit only `safe_tool`.
        let engine = AuthzEngine::from_records(&[record(
            "scoped",
            r#"permit(principal, action, resource == Route::"safe_tool");"#,
        )])
        .expect("compiles");
        let mut resp = list_tools_response();
        let filtered = filter_list_tools_response(&mut resp, &engine, "alice", "r1");
        assert!(filtered, "should recognize a tools/list result");
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "safe_tool");
    }

    #[test]
    fn filter_empty_when_nothing_permitted() {
        let engine = AuthzEngine::empty();
        let mut resp = list_tools_response();
        assert!(filter_list_tools_response(
            &mut resp, &engine, "alice", "r1"
        ));
        assert!(resp["result"]["tools"].as_array().unwrap().is_empty());
    }

    #[test]
    fn filter_keeps_all_when_permit_all() {
        let engine = engine_permit_all();
        let mut resp = list_tools_response();
        assert!(filter_list_tools_response(
            &mut resp, &engine, "alice", "r1"
        ));
        assert_eq!(resp["result"]["tools"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn filter_removes_nameless_tool_fail_closed() {
        let engine = engine_permit_all();
        let mut resp = json!({
            "result": { "tools": [ {"name": "ok"}, {"description": "no name"} ] }
        });
        assert!(filter_list_tools_response(
            &mut resp, &engine, "alice", "r1"
        ));
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1, "nameless tool removed (fail-closed)");
        assert_eq!(tools[0]["name"], "ok");
    }

    #[test]
    fn filter_ignores_non_listing_shapes() {
        let engine = engine_permit_all();
        // A request (no result.tools) — untouched.
        let mut req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call"});
        assert!(!filter_list_tools_response(
            &mut req, &engine, "alice", "r1"
        ));
        // An error response — untouched.
        let mut err = json!({"jsonrpc":"2.0","id":1,"error":{"code":-1}});
        assert!(!filter_list_tools_response(
            &mut err, &engine, "alice", "r1"
        ));
    }

    #[test]
    fn filter_body_roundtrips_and_filters() {
        let engine = AuthzEngine::from_records(&[record(
            "scoped",
            r#"permit(principal, action, resource == Route::"safe_tool");"#,
        )])
        .expect("compiles");
        let body = serde_json::to_vec(&list_tools_response()).unwrap();
        let out = filter_list_tools_body(&body, &engine, "alice", "r1").expect("is a listing");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "safe_tool");
    }

    #[test]
    fn filter_body_returns_none_for_non_json_or_non_listing() {
        let engine = engine_permit_all();
        assert!(filter_list_tools_body(b"not json", &engine, "alice", "r1").is_none());
        let other = serde_json::to_vec(&json!({"hello": "world"})).unwrap();
        assert!(filter_list_tools_body(&other, &engine, "alice", "r1").is_none());
    }

    // ── H2: fail-closed recognition of tools/list responses ──────────────────

    #[test]
    fn filter_malformed_tools_payload_is_stripped_not_leaked() {
        // A tools/list RESPONSE (has result.tools) but `tools` is NOT an array.
        // Must be recognized and stripped to empty — never forwarded intact.
        let engine = engine_permit_all();
        let mut resp = json!({
            "jsonrpc":"2.0","id":1,
            "result": { "tools": {"unexpected":"object-not-array"} }
        });
        assert!(
            filter_list_tools_response(&mut resp, &engine, "alice", "r1"),
            "recognized as a tools/list response"
        );
        assert_eq!(
            resp["result"]["tools"],
            json!([]),
            "malformed tools payload stripped to empty (fail-closed)"
        );
    }

    #[test]
    fn filter_handles_jsonrpc_batch_listing() {
        // A JSON-RPC batch: one tools/list response + one unrelated response.
        let engine = AuthzEngine::from_records(&[record(
            "scoped",
            r#"permit(principal, action, resource == Route::"safe_tool");"#,
        )])
        .expect("compiles");
        let mut batch = json!([
            {"jsonrpc":"2.0","id":1,"result":{"tools":[
                {"name":"safe_tool"},{"name":"danger_tool"}
            ]}},
            {"jsonrpc":"2.0","id":2,"result":{"other":"stuff"}}
        ]);
        assert!(
            filter_list_tools_response(&mut batch, &engine, "alice", "r1"),
            "batch containing a listing is recognized"
        );
        let first = &batch[0]["result"]["tools"];
        assert_eq!(first.as_array().unwrap().len(), 1);
        assert_eq!(first[0]["name"], "safe_tool");
        // The unrelated element is untouched.
        assert_eq!(batch[1]["result"]["other"], "stuff");
    }

    #[test]
    fn filter_batch_without_any_listing_is_not_recognized() {
        let engine = engine_permit_all();
        let mut batch = json!([
            {"jsonrpc":"2.0","id":1,"result":{"other":"x"}},
            {"jsonrpc":"2.0","id":2,"error":{"code":-1}}
        ]);
        assert!(
            !filter_list_tools_response(&mut batch, &engine, "alice", "r1"),
            "a batch with no listing is left untouched"
        );
    }

    #[test]
    fn filter_malformed_tools_via_body_is_stripped() {
        let engine = engine_permit_all();
        let body = serde_json::to_vec(&json!({
            "result": { "tools": 12345 }
        }))
        .unwrap();
        let out = filter_list_tools_body(&body, &engine, "alice", "r1")
            .expect("recognized as a (malformed) listing");
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["result"]["tools"], json!([]));
    }
}
