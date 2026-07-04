/// A2UI (Agent-to-UI) intent-driven protocol types and processing.
///
/// A2UI events are SSE frames with an `intent` field that commands the
/// frontend to perform actions (render components, navigate, show modals, etc.)
use crate::approval::{ApprovalDecision, ApprovalError, ApprovalManager};
use crate::authz::{ApprovalContext, AuthzDecision, ToolAuthzContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

/// Known A2UI intent type strings.
pub const INTENT_RENDER_COMPONENT: &str = "render_component";
pub const INTENT_UPDATE_STATE: &str = "update_state";
pub const INTENT_NAVIGATE: &str = "navigate";
pub const INTENT_SHOW_MODAL: &str = "show_modal";
pub const INTENT_SHOW_TOAST: &str = "show_toast";
pub const INTENT_REQUEST_INPUT: &str = "request_input";
pub const INTENT_STREAM_CONTENT: &str = "stream_content";

/// Synthetic A2UI intent emitted by the gate when a tool call requires human
/// approval. Frontends that understand A2UI can render an approval prompt for
/// this intent.
pub const INTENT_GATE_APPROVAL_REQUEST: &str = "gate:approval_request";

/// A parsed A2UI event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UiEvent {
    /// The intent type (e.g. `render_component`).
    pub intent: String,
    /// All other fields from the JSON payload.
    #[serde(flatten)]
    pub payload: Value,
}

impl A2UiEvent {
    /// Parse an A2UI event from a JSON string.
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// Serialize back to a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Inject `_theme` into `render_component` payloads.
    pub fn inject_theme(&mut self, theme: Value) {
        if self.intent == INTENT_RENDER_COMPONENT {
            if let Value::Object(ref mut map) = self.payload {
                map.insert("_theme".to_string(), theme);
            }
        }
    }

    /// If this A2UI intent embeds a tool invocation, return `(tool_name, args)`.
    ///
    /// The canonical A2UI intent set (`render_component`, `navigate`, … ) is
    /// UI-directive and invokes no backend tool, so this returns `None` for
    /// those — Task 4 of `add-per-tool-authz` is a documented no-op for the
    /// current protocol. It is written tolerantly so that if an emitter embeds
    /// a tool call under a `tool_name`/`tool` field (with optional `arguments`),
    /// the gate authorizes it the same way as an AG-UI tool call rather than
    /// silently passing it through.
    pub fn embedded_tool_call(&self) -> Option<(&str, &Value)> {
        const NULL: &Value = &Value::Null;
        let name = self
            .payload
            .get("tool_name")
            .or_else(|| self.payload.get("toolName"))
            .or_else(|| self.payload.get("tool"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())?;
        let args = self
            .payload
            .get("arguments")
            .or_else(|| self.payload.get("args"))
            .unwrap_or(NULL);
        Some((name, args))
    }
}

/// Scope required for each intent type.
pub fn required_scope(intent: &str) -> &'static str {
    match intent {
        INTENT_RENDER_COMPONENT => "a2ui:render",
        INTENT_UPDATE_STATE => "a2ui:state",
        INTENT_NAVIGATE => "a2ui:navigate",
        INTENT_SHOW_MODAL => "a2ui:modal",
        INTENT_SHOW_TOAST => "a2ui:toast",
        INTENT_REQUEST_INPUT => "a2ui:input",
        INTENT_STREAM_CONTENT => "a2ui:stream",
        _ => "a2ui:unknown",
    }
}

/// Handle used by the A2UI processor to register approvals and emit decision
/// notifications back to the stream task.
#[derive(Clone)]
struct ApprovalHandle {
    manager: ApprovalManager,
    decision_tx: UnboundedSender<(String, ApprovalDecision)>,
}

impl ApprovalHandle {
    /// Register a new approval request. Returns the approval id on success.
    fn request(&self, context: ApprovalContext) -> Result<String, ApprovalError> {
        let id = context.approval_id.clone();
        let ttl = context
            .expires_at
            .signed_duration_since(chrono::Utc::now())
            .to_std()
            .unwrap_or(crate::approval::DEFAULT_APPROVAL_TTL);
        let expires_at = Instant::now() + ttl;
        self.manager
            .register(id.clone(), expires_at, self.decision_tx.clone())?;
        Ok(id)
    }
}

/// An A2UI event that is being held pending human approval.
#[derive(Clone)]
struct PendingA2UiApproval {
    held_event: A2UiEvent,
    context: ApprovalContext,
}

/// Processes A2UI events: filters by intent and scope.
#[derive(Clone)]
pub struct A2UiProcessor {
    allowed_intents: Option<HashSet<String>>,
    /// Optional per-tool-call authorization, applied to any A2UI intent that
    /// embeds a tool invocation (see [`A2UiEvent::embedded_tool_call`]). `None`
    /// for routes without authz — the common path, where A2UI intents are
    /// UI-directive and carry no tool call.
    tool_authz: Option<ToolAuthzContext>,
    /// Optional handle to request human approvals. When absent, any
    /// `RequireApproval` decision is treated as a deny (fail-closed).
    approval_handle: Option<ApprovalHandle>,
    /// A single A2UI event awaiting human approval. A2UI events are stateless
    /// and one-at-a-time within the stream, so at most one approval is pending
    /// per processor.
    pending_approval: RefCell<Option<PendingA2UiApproval>>,
}

impl A2UiProcessor {
    pub fn new(allowed_intents: Vec<String>) -> Self {
        let allowed = if !allowed_intents.is_empty() {
            Some(allowed_intents.into_iter().collect())
        } else {
            None
        };
        Self {
            allowed_intents: allowed,
            tool_authz: None,
            approval_handle: None,
            pending_approval: RefCell::new(None),
        }
    }

    /// Attach a per-tool-call authorization context (builder-style).
    pub fn with_tool_authz(mut self, ctx: Option<ToolAuthzContext>) -> Self {
        self.tool_authz = ctx;
        self
    }

    /// Attach an approval handle so the processor can request human-in-the-loop
    /// decisions. Builder-style.
    pub fn with_approval_handle(
        mut self,
        manager: ApprovalManager,
        decision_tx: UnboundedSender<(String, ApprovalDecision)>,
    ) -> Self {
        self.approval_handle = Some(ApprovalHandle {
            manager,
            decision_tx,
        });
        self
    }

    /// Process an A2UI event, applying intent filtering and optional scope check.
    ///
    /// Returns `None` if the event should be dropped or held for approval.
    pub fn process(
        &self,
        mut event: A2UiEvent,
        user_scopes: &[String],
        theme: Option<Value>,
    ) -> Option<A2UiEvent> {
        // Filter by allowed intents
        if let Some(allowed) = &self.allowed_intents {
            if !allowed.contains(&event.intent) {
                tracing::debug!(intent = %event.intent, "A2UI event blocked by intent allow-list");
                return None;
            }
        }

        // Scope check (if user has scopes defined)
        if !user_scopes.is_empty() && !self.check_scope(&event.intent, user_scopes) {
            tracing::debug!(
                intent = %event.intent,
                "A2UI event blocked by scope check"
            );
            return None;
        }

        // Inject theme for render_component before any hold, so the buffered
        // event is ready to forward as-is on approval.
        if let Some(theme_value) = theme {
            event.inject_theme(theme_value);
        }

        // Task 4: per-tool authorization for A2UI intents that embed a tool
        // invocation. The canonical intent set embeds none (this is a no-op for
        // them), but a tool-bearing intent is authorized like an AG-UI tool
        // call and dropped on Deny (fail-closed). RequireApproval pauses the
        // stream and emits an approval request.
        if let Some(authz) = &self.tool_authz {
            if let Some((tool_name, args)) = event.clone().embedded_tool_call() {
                match authz.authorize(tool_name, args) {
                    AuthzDecision::Allow => {}
                    AuthzDecision::RequireApproval(ctx) => {
                        return self.request_approval(event, ctx, tool_name);
                    }
                    AuthzDecision::Deny => {
                        tracing::info!(
                            intent = %event.intent,
                            tool = %tool_name,
                            "A2UI embedded tool call denied by policy — dropping"
                        );
                        return None;
                    }
                }
            }
        }

        Some(event)
    }

    /// Ids of approval requests currently pending for this stream.
    pub fn pending_approval_ids(&self) -> Vec<String> {
        self.pending_approval
            .borrow()
            .as_ref()
            .map(|p| vec![p.context.approval_id.clone()])
            .unwrap_or_default()
    }

    /// Metadata for any approval request currently pending.
    pub fn approval_contexts(&self) -> Vec<ApprovalContext> {
        self.pending_approval
            .borrow()
            .as_ref()
            .map(|p| p.context.clone())
            .into_iter()
            .collect()
    }

    /// Build the synthetic approval-request event for the current pending
    /// approval, if any. The caller uses this to emit the request over the
    /// stream while upstream reads are paused.
    pub fn approval_request_event(&self) -> Option<A2UiEvent> {
        let pending = self.pending_approval.borrow();
        pending
            .as_ref()
            .map(|p| build_approval_request_event(&p.context, &p.held_event.intent))
    }

    /// Resolve a pending approval. Returns the held A2UI event on approve, or
    /// `None` on deny (or if the id does not match the pending approval).
    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Option<A2UiEvent> {
        let pending = self.pending_approval.borrow_mut().take()?;
        if pending.context.approval_id != approval_id {
            // Not the approval we are holding; restore it.
            self.pending_approval.borrow_mut().replace(pending);
            return None;
        }

        match decision {
            ApprovalDecision::Approve => {
                tracing::info!(
                    approval_id,
                    intent = %pending.held_event.intent,
                    "human approval granted — resuming A2UI event"
                );
                Some(pending.held_event)
            }
            ApprovalDecision::Deny => {
                tracing::info!(
                    approval_id,
                    intent = %pending.held_event.intent,
                    "human approval denied — dropping A2UI event"
                );
                None
            }
        }
    }

    fn request_approval(
        &self,
        event: A2UiEvent,
        context: ApprovalContext,
        tool_name: &str,
    ) -> Option<A2UiEvent> {
        // A2UI approvals are single-event; if one is already pending, the new
        // event is dropped. This is consistent with fail-closed pause behavior.
        if self.pending_approval.borrow().is_some() {
            tracing::warn!(
                intent = %event.intent,
                tool = %tool_name,
                "A2UI approval already pending — dropping new tool call"
            );
            return None;
        }

        let Some(handle) = &self.approval_handle else {
            tracing::warn!(
                intent = %event.intent,
                tool = %tool_name,
                "approval required but no approval manager configured — denying (fail-closed)"
            );
            return None;
        };

        let id = context.approval_id.clone();
        if let Err(e) = handle.request(context.clone()) {
            tracing::error!(
                approval_id = %id,
                error = %e,
                "failed to register A2UI approval request — denying (fail-closed)"
            );
            return None;
        }

        self.pending_approval
            .borrow_mut()
            .replace(PendingA2UiApproval {
                held_event: event,
                context,
            });

        tracing::info!(
            approval_id = %id,
            intent = %self.pending_approval.borrow().as_ref().unwrap().held_event.intent,
            tool = %tool_name,
            "A2UI tool call requires human approval — pausing stream"
        );
        None
    }

    /// Check whether any of the user's scopes permit the given intent.
    fn check_scope(&self, intent: &str, user_scopes: &[String]) -> bool {
        let required = required_scope(intent);
        for scope in user_scopes {
            if scope == "a2ui:*" || scope == required {
                return true;
            }
        }
        false
    }

    /// Filter events by scope from a comma-separated scope string.
    pub fn filter_by_scope(
        &self,
        event: A2UiEvent,
        scope_string: &str,
        theme: Option<Value>,
    ) -> Option<A2UiEvent> {
        let scopes: Vec<String> = scope_string
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.process(event, &scopes, theme)
    }
}

/// Build a synthetic A2UI `gate:approval_request` event asking a human operator
/// to approve an A2UI-embedded tool call.
fn build_approval_request_event(context: &ApprovalContext, original_intent: &str) -> A2UiEvent {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "approvalId".to_string(),
        Value::String(context.approval_id.clone()),
    );
    payload.insert(
        "principalId".to_string(),
        Value::String(context.principal_id.clone()),
    );
    payload.insert("action".to_string(), Value::String(context.action.clone()));
    payload.insert(
        "resourceId".to_string(),
        Value::String(context.resource_id.clone()),
    );
    payload.insert(
        "expiresAt".to_string(),
        Value::String(context.expires_at.to_rfc3339()),
    );
    payload.insert(
        "intent".to_string(),
        Value::String(original_intent.to_string()),
    );
    if let Some(reason) = &context.reason {
        payload.insert("reason".to_string(), Value::String(reason.clone()));
    }
    A2UiEvent {
        intent: INTENT_GATE_APPROVAL_REQUEST.to_string(),
        payload: Value::Object(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_render_component() {
        let json = r#"{"intent":"render_component","component":"ChatWidget","props":{}}"#;
        let event = A2UiEvent::from_json(json).unwrap();
        assert_eq!(event.intent, "render_component");
    }

    #[test]
    fn processor_blocks_unlisted_intent() {
        let proc = A2UiProcessor::new(vec!["show_toast".to_string()]);
        let event = A2UiEvent {
            intent: INTENT_NAVIGATE.to_string(),
            payload: json!({"to": "/dashboard"}),
        };
        assert!(proc.process(event, &[], None).is_none());
    }

    #[test]
    fn processor_allows_listed_intent() {
        let proc = A2UiProcessor::new(vec!["navigate".to_string()]);
        let event = A2UiEvent {
            intent: INTENT_NAVIGATE.to_string(),
            payload: json!({"to": "/home"}),
        };
        assert!(proc.process(event, &[], None).is_some());
    }

    #[test]
    fn scope_wildcard_passes() {
        let proc = A2UiProcessor::new(vec![]);
        let event = A2UiEvent {
            intent: INTENT_RENDER_COMPONENT.to_string(),
            payload: json!({}),
        };
        let scopes = vec!["a2ui:*".to_string()];
        assert!(proc.process(event, &scopes, None).is_some());
    }

    #[test]
    fn scope_specific_passes() {
        let proc = A2UiProcessor::new(vec![]);
        let event = A2UiEvent {
            intent: INTENT_NAVIGATE.to_string(),
            payload: json!({"to": "/x"}),
        };
        let scopes = vec!["a2ui:navigate".to_string()];
        assert!(proc.process(event, &scopes, None).is_some());
    }

    #[test]
    fn scope_mismatch_blocks() {
        let proc = A2UiProcessor::new(vec![]);
        let event = A2UiEvent {
            intent: INTENT_NAVIGATE.to_string(),
            payload: json!({"to": "/x"}),
        };
        let scopes = vec!["a2ui:render".to_string()];
        assert!(proc.process(event, &scopes, None).is_none());
    }

    #[test]
    fn injects_theme_for_render_component() {
        let proc = A2UiProcessor::new(vec![]);
        let event = A2UiEvent {
            intent: INTENT_RENDER_COMPONENT.to_string(),
            payload: json!({"component": "Widget"}),
        };
        let theme = json!({"primaryColor": "#ff0000"});
        let result = proc.process(event, &[], Some(theme)).unwrap();
        assert_eq!(result.payload["_theme"]["primaryColor"], "#ff0000");
    }

    // ── Task 4: A2UI embedded tool-call authorization ───────────────────────

    use crate::authz::{AuthzEngine, PolicyRecord, ToolAuthzContext};
    use std::sync::Arc;

    fn record(id: &str, text: &str) -> PolicyRecord {
        PolicyRecord {
            id: id.to_string(),
            policy_text: text.to_string(),
            schema_json: None,
            entities_json: None,
        }
    }

    fn ctx(policy: &str) -> ToolAuthzContext {
        ToolAuthzContext {
            engine: Arc::new(AuthzEngine::from_records(&[record("p", policy)]).expect("compiles")),
            principal_kind: crate::authz::PrincipalKind::User,
            revoked: false,
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        }
    }

    #[test]
    fn canonical_intents_have_no_embedded_tool_call() {
        // The documented no-op: standard UI-directive intents invoke no tool.
        for intent in [
            INTENT_RENDER_COMPONENT,
            INTENT_NAVIGATE,
            INTENT_SHOW_MODAL,
            INTENT_SHOW_TOAST,
        ] {
            let ev = A2UiEvent {
                intent: intent.to_string(),
                payload: json!({"component": "X", "to": "/y"}),
            };
            assert!(
                ev.embedded_tool_call().is_none(),
                "{intent} must not be treated as a tool call"
            );
        }
    }

    #[test]
    fn embedded_tool_call_extracted_from_payload() {
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "search", "arguments": {"q": "x"}}),
        };
        let (name, args) = ev.embedded_tool_call().expect("has a tool call");
        assert_eq!(name, "search");
        assert_eq!(args["q"], "x");
    }

    #[test]
    fn a2ui_denies_embedded_tool_call_when_policy_denies() {
        // Deny-all engine → an intent that embeds a tool call is dropped.
        let proc = A2UiProcessor::new(vec![]).with_tool_authz(Some(ToolAuthzContext {
            engine: Arc::new(AuthzEngine::empty()),
            principal_kind: crate::authz::PrincipalKind::User,
            revoked: false,
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        }));
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "danger"}),
        };
        assert!(proc.process(ev, &[], None).is_none());
    }

    #[test]
    fn a2ui_allows_embedded_tool_call_when_policy_permits() {
        let proc = A2UiProcessor::new(vec![])
            .with_tool_authz(Some(ctx("permit(principal, action, resource);")));
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "ok"}),
        };
        assert!(proc.process(ev, &[], None).is_some());
    }

    #[test]
    fn a2ui_canonical_intents_unaffected_by_tool_authz() {
        // A deny-all engine must NOT block ordinary UI intents (no tool call).
        let proc = A2UiProcessor::new(vec![]).with_tool_authz(Some(ToolAuthzContext {
            engine: Arc::new(AuthzEngine::empty()),
            principal_kind: crate::authz::PrincipalKind::User,
            revoked: false,
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        }));
        let ev = A2UiEvent {
            intent: INTENT_NAVIGATE.to_string(),
            payload: json!({"to": "/home"}),
        };
        assert!(
            proc.process(ev, &[], None).is_some(),
            "UI-directive intents are not tool calls and must pass"
        );
    }

    // ── add-hitl-approval: A2UI embedded tool-call approval ───────────────

    #[test]
    fn a2ui_require_approval_holds_event_and_emits_request() {
        let policy = r#"@require_approval("Sensitive A2UI tool")
            permit(principal, action, resource);"#;
        let proc = A2UiProcessor::new(vec![])
            .with_tool_authz(Some(ctx(policy)))
            .with_approval_handle(
                ApprovalManager::new(),
                tokio::sync::mpsc::unbounded_channel().0,
            );
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "danger", "arguments": {"x": 1}}),
        };

        // Event is held pending approval.
        assert!(proc.process(ev.clone(), &[], None).is_none());

        let ids = proc.pending_approval_ids();
        assert_eq!(ids.len(), 1);

        let req = proc
            .approval_request_event()
            .expect("approval request emitted");
        assert_eq!(req.intent, INTENT_GATE_APPROVAL_REQUEST);
        assert_eq!(req.payload["intent"], "invoke_tool");
        assert_eq!(req.payload["action"], "call_tool");
        assert_eq!(req.payload["resourceId"], "danger");
        assert_eq!(req.payload["approvalId"], ids[0]);
    }

    #[test]
    fn a2ui_approve_releases_held_event() {
        let policy = r#"@require_approval("")
            permit(principal, action, resource);"#;
        let proc = A2UiProcessor::new(vec![])
            .with_tool_authz(Some(ctx(policy)))
            .with_approval_handle(
                ApprovalManager::new(),
                tokio::sync::mpsc::unbounded_channel().0,
            );
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "ok"}),
        };

        assert!(proc.process(ev.clone(), &[], None).is_none());
        let id = proc.pending_approval_ids()[0].clone();

        let released = proc.resolve_approval(&id, ApprovalDecision::Approve);
        let released = released.expect("event released on approve");
        assert_eq!(released.intent, "invoke_tool");
        assert!(proc.pending_approval_ids().is_empty());
    }

    #[test]
    fn a2ui_deny_drops_held_event() {
        let policy = r#"@require_approval("no")
            permit(principal, action, resource);"#;
        let proc = A2UiProcessor::new(vec![])
            .with_tool_authz(Some(ctx(policy)))
            .with_approval_handle(
                ApprovalManager::new(),
                tokio::sync::mpsc::unbounded_channel().0,
            );
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "nope"}),
        };

        assert!(proc.process(ev, &[], None).is_none());
        let id = proc.pending_approval_ids()[0].clone();

        assert!(proc.resolve_approval(&id, ApprovalDecision::Deny).is_none());
        assert!(proc.pending_approval_ids().is_empty());
    }

    #[test]
    fn a2ui_require_approval_without_handle_fails_closed() {
        let policy = r#"@require_approval("")
            permit(principal, action, resource);"#;
        let proc = A2UiProcessor::new(vec![]).with_tool_authz(Some(ctx(policy)));
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "ok"}),
        };

        assert!(proc.process(ev, &[], None).is_none());
        assert!(proc.pending_approval_ids().is_empty());
    }

    #[test]
    fn a2ui_resolve_wrong_id_leaves_pending_intact() {
        let policy = r#"@require_approval("")
            permit(principal, action, resource);"#;
        let proc = A2UiProcessor::new(vec![])
            .with_tool_authz(Some(ctx(policy)))
            .with_approval_handle(
                ApprovalManager::new(),
                tokio::sync::mpsc::unbounded_channel().0,
            );
        let ev = A2UiEvent {
            intent: "invoke_tool".to_string(),
            payload: json!({"tool_name": "ok"}),
        };

        assert!(proc.process(ev, &[], None).is_none());
        assert_eq!(proc.pending_approval_ids().len(), 1);

        assert!(proc
            .resolve_approval("wrong-id", ApprovalDecision::Approve)
            .is_none());
        assert_eq!(proc.pending_approval_ids().len(), 1);
    }
}
