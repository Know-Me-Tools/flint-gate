/// AG-UI (CopilotKit Agent-User Interface) protocol types and processing.
///
/// AG-UI events are delivered as SSE frames. Each frame's `data:` field
/// contains a JSON object with a `type` field identifying the event.
use crate::authz::ToolAuthzContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// All known AG-UI event type strings.
pub const EVENT_TEXT_MESSAGE_START: &str = "TEXT_MESSAGE_START";
pub const EVENT_TEXT_MESSAGE_CONTENT: &str = "TEXT_MESSAGE_CONTENT";
pub const EVENT_TEXT_MESSAGE_END: &str = "TEXT_MESSAGE_END";
pub const EVENT_TOOL_CALL_START: &str = "TOOL_CALL_START";
pub const EVENT_TOOL_CALL_ARGS: &str = "TOOL_CALL_ARGS";
pub const EVENT_TOOL_CALL_END: &str = "TOOL_CALL_END";
pub const EVENT_STATE_SNAPSHOT: &str = "STATE_SNAPSHOT";
pub const EVENT_STATE_DELTA: &str = "STATE_DELTA";
pub const EVENT_MESSAGES_SNAPSHOT: &str = "MESSAGES_SNAPSHOT";
pub const EVENT_RUN_STARTED: &str = "RUN_STARTED";
pub const EVENT_RUN_FINISHED: &str = "RUN_FINISHED";
pub const EVENT_RUN_ERROR: &str = "RUN_ERROR";
pub const EVENT_STEP_STARTED: &str = "STEP_STARTED";
pub const EVENT_STEP_FINISHED: &str = "STEP_FINISHED";
pub const EVENT_RAW: &str = "RAW";

/// A parsed AG-UI event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgUiEvent {
    /// The event type string (e.g. `TEXT_MESSAGE_CONTENT`).
    #[serde(rename = "type")]
    pub event_type: String,
    /// All other fields from the JSON payload.
    #[serde(flatten)]
    pub payload: Value,
}

impl AgUiEvent {
    /// Parse an AG-UI event from a JSON string.
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// Classify the event type into one of the known variants.
    pub fn classify(&self) -> AgUiEventType {
        match self.event_type.as_str() {
            EVENT_TEXT_MESSAGE_START => AgUiEventType::TextMessageStart,
            EVENT_TEXT_MESSAGE_CONTENT => AgUiEventType::TextMessageContent,
            EVENT_TEXT_MESSAGE_END => AgUiEventType::TextMessageEnd,
            EVENT_TOOL_CALL_START => AgUiEventType::ToolCallStart,
            EVENT_TOOL_CALL_ARGS => AgUiEventType::ToolCallArgs,
            EVENT_TOOL_CALL_END => AgUiEventType::ToolCallEnd,
            EVENT_STATE_SNAPSHOT => AgUiEventType::StateSnapshot,
            EVENT_STATE_DELTA => AgUiEventType::StateDelta,
            EVENT_MESSAGES_SNAPSHOT => AgUiEventType::MessagesSnapshot,
            EVENT_RUN_STARTED => AgUiEventType::RunStarted,
            EVENT_RUN_FINISHED => AgUiEventType::RunFinished,
            EVENT_RUN_ERROR => AgUiEventType::RunError,
            EVENT_STEP_STARTED => AgUiEventType::StepStarted,
            EVENT_STEP_FINISHED => AgUiEventType::StepFinished,
            EVENT_RAW => AgUiEventType::Raw,
            other => AgUiEventType::Unknown(other.to_string()),
        }
    }

    /// Inject `_gate_metadata` into the event payload.
    pub fn inject_metadata(&mut self, metadata: serde_json::Map<String, Value>) {
        if let Value::Object(ref mut map) = self.payload {
            map.insert("_gate_metadata".to_string(), Value::Object(metadata));
        }
    }

    /// Serialize back to a JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The tool-call id correlating a `TOOL_CALL_START` / `ARGS` / `END` triple.
    ///
    /// AG-UI carries this as `toolCallId`; some emitters use snake_case
    /// (`tool_call_id`). Accept either so the gate is robust in front of
    /// varying implementations.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.payload
            .get("toolCallId")
            .or_else(|| self.payload.get("tool_call_id"))
            .and_then(Value::as_str)
    }

    /// The tool name carried on a `TOOL_CALL_START` event (`toolCallName`, or
    /// snake_case `tool_call_name`).
    pub fn tool_call_name(&self) -> Option<&str> {
        self.payload
            .get("toolCallName")
            .or_else(|| self.payload.get("tool_call_name"))
            .and_then(Value::as_str)
    }

    /// The argument-fragment string carried on a `TOOL_CALL_ARGS` delta event.
    ///
    /// AG-UI streams tool arguments as a sequence of string deltas (each a
    /// partial of the eventual JSON args object) under `delta`.
    pub fn tool_call_args_delta(&self) -> Option<&str> {
        self.payload.get("delta").and_then(Value::as_str)
    }
}

/// Classified AG-UI event types.
#[derive(Debug, Clone, PartialEq)]
pub enum AgUiEventType {
    TextMessageStart,
    TextMessageContent,
    TextMessageEnd,
    ToolCallStart,
    ToolCallArgs,
    ToolCallEnd,
    StateSnapshot,
    StateDelta,
    MessagesSnapshot,
    RunStarted,
    RunFinished,
    RunError,
    StepStarted,
    StepFinished,
    Raw,
    Unknown(String),
}

/// Estimates tokens from AG-UI `TEXT_MESSAGE_CONTENT` delta events.
///
/// Uses a simple `chars / 4` heuristic — good enough for billing estimates.
#[derive(Debug, Default)]
pub struct AgUiTokenCounter {
    total_chars: usize,
}

impl AgUiTokenCounter {
    /// Count characters from a `TEXT_MESSAGE_CONTENT` event's `delta` field.
    pub fn count_event(&mut self, event: &AgUiEvent) {
        if event.event_type == EVENT_TEXT_MESSAGE_CONTENT {
            if let Some(delta) = event.payload.get("delta").and_then(|d| d.as_str()) {
                self.total_chars += delta.len();
            }
        }
    }

    /// Estimated token count (chars / 4).
    pub fn estimated_tokens(&self) -> u64 {
        (self.total_chars as u64).saturating_add(3) / 4
    }
}

/// Per-tool-call authorization state, tracked by tool-call id across the
/// `TOOL_CALL_START` → `ARGS`* → `END` sequence.
///
/// **Buffer-until-authorized (inspect-then-forward).** AG-UI clients execute a
/// tool only after `TOOL_CALL_END`; the intervening `ARGS` deltas are
/// display-only. So the gate holds the entire tool call — the START event and
/// every ARGS delta — emitting NOTHING downstream until the complete call is
/// authorized at END. On allow it flushes the held START, a single coalesced
/// ARGS carrying the full arguments, then END, in order. On deny it emits only
/// a synthetic RUN_ERROR. This is the industry-standard posture (agentgateway,
/// Kong, Portkey, LangGraph): never forward-then-annul, and never leak argument
/// bytes for a call that is ultimately blocked.
#[derive(Clone)]
struct ToolCallState {
    /// The tool name from `TOOL_CALL_START`, used for authorization at END.
    tool_name: String,
    /// The held `TOOL_CALL_START` event, flushed only on an END-allow. `None`
    /// once the state is marked blocked (its START was denied and dropped).
    held_start: Option<AgUiEvent>,
    /// Whether the coarse by-name START check allowed the call. When `false`
    /// the id is blocked: its START was already replaced with a RUN_ERROR and
    /// all further ARGS/END for the id are dropped.
    start_allowed: bool,
    /// Accumulated argument-delta fragments, concatenated in arrival order, to
    /// authorize on the *complete* args at END. Capped by `max_tool_args_bytes`.
    args_buffer: String,
}

/// Processes AG-UI events: validates against the allowed list, injects
/// metadata, and (when a [`ToolAuthzContext`] is present) authorizes tool calls
/// by buffering each call until END, then releasing or blocking it.
#[derive(Clone)]
pub struct AgUiProcessor {
    allowed_events: Option<HashSet<String>>,
    validate: bool,
    /// Optional per-tool-call authorization. `None` → tool calls stream live,
    /// unauthorized (backward-compatible for routes without authz).
    tool_authz: Option<ToolAuthzContext>,
    /// Byte cap on a single tool call's accumulated arguments held pending
    /// authorization (C1 DoS guard). Exceeding it denies that call.
    max_tool_args_bytes: usize,
    /// In-flight tool-call state keyed by tool-call id. Interior mutability:
    /// `process_multi(&self, …)` runs single-threaded inside one stream's
    /// processor task, so a `RefCell` is sufficient and lock-free.
    tool_calls: RefCell<HashMap<String, ToolCallState>>,
}

impl AgUiProcessor {
    pub fn new(validate: bool, allowed_events: Vec<String>) -> Self {
        let allowed = if validate && !allowed_events.is_empty() {
            Some(allowed_events.into_iter().collect())
        } else {
            None
        };
        Self {
            allowed_events: allowed,
            validate,
            tool_authz: None,
            max_tool_args_bytes: crate::stream::DEFAULT_MAX_TOOL_ARGS_BYTES,
            tool_calls: RefCell::new(HashMap::new()),
        }
    }

    /// Attach a per-tool-call authorization context. Builder-style so existing
    /// construction sites are unchanged; only routes that thread a context get
    /// per-tool authorization.
    pub fn with_tool_authz(mut self, ctx: Option<ToolAuthzContext>) -> Self {
        self.tool_authz = ctx;
        self
    }

    /// Override the per-tool-call args byte cap (C1). Builder-style.
    pub fn with_max_tool_args_bytes(mut self, cap: usize) -> Self {
        if cap > 0 {
            self.max_tool_args_bytes = cap;
        }
        self
    }

    /// Process an event, returning **zero or more** events to forward, in order.
    ///
    /// This is the primary entry point. Most events map 1→1 (or 1→0 when
    /// dropped), but a `TOOL_CALL_END` that authorizes a buffered call releases
    /// several held events at once (START + coalesced ARGS + END) — hence a
    /// `Vec`. Non-tool events (notably `TEXT_MESSAGE_CONTENT`) are NEVER
    /// buffered: they pass straight through and stream live with no added
    /// latency. Only tool-call events are held.
    ///
    /// Order of operations: allow-list validation first (cheapest, and a
    /// disallowed event is dropped regardless of authz), then per-tool-call
    /// authorization, then metadata injection on whatever survives.
    pub fn process_multi(
        &self,
        event: AgUiEvent,
        metadata: serde_json::Map<String, Value>,
    ) -> Vec<AgUiEvent> {
        // Validate against allowed list (unchanged behavior).
        if self.validate {
            if let Some(allowed) = &self.allowed_events {
                if !allowed.contains(&event.event_type) {
                    tracing::debug!(event_type = %event.event_type, "AG-UI event blocked by allow-list");
                    return Vec::new();
                }
            }
        }

        let released = self.authorize_tool_event(event);
        released
            .into_iter()
            .map(|ev| inject_meta(ev, metadata.clone()))
            .collect()
    }

    /// Back-compat single-event wrapper. Returns the first released event (or
    /// `None`). Only correct for callers that never pass tool-call events whose
    /// END releases multiple events — retained for tests and non-tool paths.
    #[cfg(test)]
    pub fn process(
        &self,
        event: AgUiEvent,
        metadata: serde_json::Map<String, Value>,
    ) -> Option<AgUiEvent> {
        self.process_multi(event, metadata).into_iter().next()
    }

    /// Route a tool-call event through the buffer-until-authorized state
    /// machine, mutating per-id state, and return the events to release now.
    ///
    /// Non-tool events, and all events when no [`ToolAuthzContext`] is present,
    /// are released immediately (single-element vec) — never buffered.
    ///
    /// Tool-call model (buffer-until-authorized; args are display-only until
    /// END so holding them adds no client-visible semantics):
    /// - `TOOL_CALL_START`: coarse by-name check. Deny → mark id blocked, emit a
    ///   RUN_ERROR, hold nothing. Allow → HOLD the START, emit nothing.
    /// - `TOOL_CALL_ARGS`: append the delta to the id's buffer (cap-checked).
    ///   Emit nothing. If the id was blocked or unknown → drop. Cap exceeded →
    ///   deny the call (drop state, emit RUN_ERROR).
    /// - `TOOL_CALL_END`: authorize on the COMPLETE buffered args (the real
    ///   gate). Allow → flush held START + one coalesced ARGS (full args) + END.
    ///   Deny → emit RUN_ERROR only; nothing of the call reaches the client.
    ///
    /// Because the forwarding boundary is END (not START), an arguments-refined
    /// policy may be authored either as `permit(by name) + forbid(when args …)`
    /// OR as `permit … unless { args … }`: the coarse START check runs with
    /// empty args purely as an early-reject optimization, and even if it lets a
    /// call through, the END check on full args is authoritative and nothing is
    /// forwarded before it. (A `permit … unless {args}` policy simply denies the
    /// coarse START check when args are absent, so such a call is blocked early
    /// — still fail-closed, never fail-open.)
    ///
    /// Fail-closed: a tool-call event missing its id (or a START missing its
    /// name) is denied/dropped, never released.
    fn authorize_tool_event(&self, event: AgUiEvent) -> Vec<AgUiEvent> {
        let Some(authz) = &self.tool_authz else {
            return vec![event];
        };

        match event.classify() {
            AgUiEventType::ToolCallStart => self.on_tool_call_start(authz, event),
            AgUiEventType::ToolCallArgs => self.on_tool_call_args(event),
            AgUiEventType::ToolCallEnd => self.on_tool_call_end(authz, event),
            // Every non-tool event (TEXT_MESSAGE_CONTENT, RUN_*, STATE_*, …)
            // streams live — never buffered.
            _ => vec![event],
        }
    }

    fn on_tool_call_start(&self, authz: &ToolAuthzContext, event: AgUiEvent) -> Vec<AgUiEvent> {
        // Fail-closed: a START with no id or no name cannot be authorized.
        let (Some(id), Some(name)) = (event.tool_call_id(), event.tool_call_name()) else {
            tracing::warn!("AG-UI TOOL_CALL_START missing id or name — denying (fail-closed)");
            return vec![run_error_event(
                event.tool_call_id(),
                "tool call rejected: malformed start event",
            )];
        };
        let (id, name) = (id.to_string(), name.to_string());

        // Coarse by-name check (no args yet) — an early-reject optimization.
        let allowed = authz.authorize(&name, &Value::Null).is_allow();

        if allowed {
            // HOLD the START; emit nothing until END authorizes the full call.
            self.tool_calls.borrow_mut().insert(
                id,
                ToolCallState {
                    tool_name: name,
                    held_start: Some(event),
                    start_allowed: true,
                    args_buffer: String::new(),
                },
            );
            Vec::new()
        } else {
            // Blocked at START: record the block so later ARGS/END drop, and
            // replace the START with a single RUN_ERROR.
            self.tool_calls.borrow_mut().insert(
                id.clone(),
                ToolCallState {
                    tool_name: name.clone(),
                    held_start: None,
                    start_allowed: false,
                    args_buffer: String::new(),
                },
            );
            tracing::info!(tool = %name, tool_call_id = %id, "tool call denied at START — blocking");
            vec![run_error_event(
                Some(&id),
                &format!("tool call `{name}` denied by policy"),
            )]
        }
    }

    fn on_tool_call_args(&self, event: AgUiEvent) -> Vec<AgUiEvent> {
        let Some(id) = event.tool_call_id().map(str::to_string) else {
            // An args delta with no id cannot be correlated — drop it.
            tracing::warn!("AG-UI TOOL_CALL_ARGS missing id — dropping (fail-closed)");
            return Vec::new();
        };
        let mut calls = self.tool_calls.borrow_mut();
        let Some(state) = calls.get_mut(&id) else {
            // Args for an unknown id (no START seen) — fail-closed: drop.
            tracing::warn!(tool_call_id = %id, "TOOL_CALL_ARGS for unknown tool call — dropping");
            return Vec::new();
        };
        if !state.start_allowed {
            // Belongs to a call denied at START — drop consistently.
            return Vec::new();
        }
        if let Some(delta) = event.tool_call_args_delta() {
            // C1: cap the accumulated args. On overflow, DENY this tool call —
            // drop its held state and emit a RUN_ERROR (do NOT tear down the
            // whole stream for one oversized call).
            if state.args_buffer.len().saturating_add(delta.len()) > self.max_tool_args_bytes {
                let tool = state.tool_name.clone();
                calls.remove(&id);
                tracing::warn!(
                    tool = %tool,
                    tool_call_id = %id,
                    cap = self.max_tool_args_bytes,
                    "tool-call args exceeded cap — denying (fail-closed)"
                );
                return vec![run_error_event(
                    Some(&id),
                    &format!("tool call `{tool}` denied: arguments exceeded size limit"),
                )];
            }
            state.args_buffer.push_str(delta);
        }
        // Hold: emit nothing until END.
        Vec::new()
    }

    fn on_tool_call_end(&self, authz: &ToolAuthzContext, event: AgUiEvent) -> Vec<AgUiEvent> {
        let Some(id) = event.tool_call_id().map(str::to_string) else {
            tracing::warn!("AG-UI TOOL_CALL_END missing id — dropping (fail-closed)");
            return Vec::new();
        };
        // Remove the state: the call is terminating either way.
        let Some(state) = self.tool_calls.borrow_mut().remove(&id) else {
            tracing::warn!(tool_call_id = %id, "TOOL_CALL_END for unknown tool call — dropping");
            return Vec::new();
        };

        if !state.start_allowed {
            // Already blocked at START; its RUN_ERROR was emitted then. Drop END.
            return Vec::new();
        }

        // Authorize on the COMPLETE accumulated arguments — the real gate.
        // L2: an EMPTY buffer is a no-arg call → {}. A NON-EMPTY buffer that
        // fails to parse means the tool sent arguments we cannot authorize →
        // DENY (fail-closed), never coerce to {} and allow.
        let args = match parse_args_buffer(&state.args_buffer) {
            Some(v) => v,
            None => {
                tracing::warn!(
                    tool = %state.tool_name,
                    tool_call_id = %id,
                    "tool-call args unparseable at END — denying (fail-closed)"
                );
                return vec![run_error_event(
                    Some(&id),
                    &format!(
                        "tool call `{}` denied: arguments could not be parsed",
                        state.tool_name
                    ),
                )];
            }
        };

        if authz.authorize(&state.tool_name, &args).is_allow() {
            // FLUSH the whole call in order: held START, one coalesced ARGS with
            // the full arguments, then END. The coalesced ARGS replaces the
            // stream of deltas the client never saw.
            let mut out = Vec::with_capacity(3);
            if let Some(start) = state.held_start {
                out.push(start);
            }
            if !state.args_buffer.trim().is_empty() {
                out.push(coalesced_args_event(&id, &state.args_buffer));
            }
            out.push(event);
            out
        } else {
            tracing::info!(
                tool = %state.tool_name,
                tool_call_id = %id,
                "tool call denied at END on full arguments — blocking"
            );
            vec![run_error_event(
                Some(&id),
                &format!("tool call `{}` denied by policy", state.tool_name),
            )]
        }
    }
}

/// Inject metadata into an event (no-op when metadata is empty).
fn inject_meta(mut event: AgUiEvent, metadata: serde_json::Map<String, Value>) -> AgUiEvent {
    if !metadata.is_empty() {
        event.inject_metadata(metadata);
    }
    event
}

/// Parse an accumulated tool-call args buffer into a JSON value.
///
/// - Empty (or whitespace-only) buffer → `Some({})` — a legitimate no-arg call.
/// - Non-empty and valid JSON → `Some(value)`.
/// - Non-empty but NOT valid JSON → `None`, which the caller treats as a DENY
///   (fail-closed): the tool sent arguments we cannot authorize, so we must not
///   allow it. (L2 fix.)
fn parse_args_buffer(buffer: &str) -> Option<Value> {
    let trimmed = buffer.trim();
    if trimmed.is_empty() {
        return Some(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(trimmed).ok()
}

/// Build the single coalesced `TOOL_CALL_ARGS` event that replaces the held
/// delta stream for an authorized call: one event carrying the full arguments
/// string under `delta`, tagged with the tool-call id.
fn coalesced_args_event(tool_call_id: &str, full_args: &str) -> AgUiEvent {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "toolCallId".to_string(),
        Value::String(tool_call_id.to_string()),
    );
    payload.insert("delta".to_string(), Value::String(full_args.to_string()));
    AgUiEvent {
        event_type: EVENT_TOOL_CALL_ARGS.to_string(),
        payload: Value::Object(payload),
    }
}

/// Build a synthetic AG-UI `RUN_ERROR` event announcing a blocked tool call.
///
/// Carries the tool-call id (when known) so a client can correlate the error
/// with the call it started. This is the "benign replacement" the client sees
/// instead of a denied call silently vanishing mid-stream.
fn run_error_event(tool_call_id: Option<&str>, message: &str) -> AgUiEvent {
    let mut payload = serde_json::Map::new();
    payload.insert("message".to_string(), Value::String(message.to_string()));
    payload.insert(
        "code".to_string(),
        Value::String("tool_call_denied".to_string()),
    );
    if let Some(id) = tool_call_id {
        payload.insert("toolCallId".to_string(), Value::String(id.to_string()));
    }
    AgUiEvent {
        event_type: EVENT_RUN_ERROR.to_string(),
        payload: Value::Object(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_text_message_content() {
        let json = r#"{"type":"TEXT_MESSAGE_CONTENT","message_id":"m1","delta":"hello"}"#;
        let event = AgUiEvent::from_json(json).unwrap();
        assert_eq!(event.event_type, "TEXT_MESSAGE_CONTENT");
        assert_eq!(event.classify(), AgUiEventType::TextMessageContent);
    }

    #[test]
    fn token_counter() {
        let mut counter = AgUiTokenCounter::default();
        let event = AgUiEvent {
            event_type: EVENT_TEXT_MESSAGE_CONTENT.to_string(),
            payload: json!({"delta": "Hello world!"}), // 12 chars
        };
        counter.count_event(&event);
        // 12 chars → 3 tokens
        assert_eq!(counter.estimated_tokens(), 3);
    }

    #[test]
    fn processor_blocks_unknown_events() {
        let processor = AgUiProcessor::new(true, vec!["TEXT_MESSAGE_CONTENT".to_string()]);
        let event = AgUiEvent {
            event_type: "TOOL_CALL_START".to_string(),
            payload: json!({}),
        };
        assert!(processor.process(event, Default::default()).is_none());
    }

    #[test]
    fn processor_allows_listed_event() {
        let processor = AgUiProcessor::new(true, vec!["TEXT_MESSAGE_CONTENT".to_string()]);
        let event = AgUiEvent {
            event_type: "TEXT_MESSAGE_CONTENT".to_string(),
            payload: json!({"delta": "hi"}),
        };
        assert!(processor.process(event, Default::default()).is_some());
    }

    #[test]
    fn inject_metadata_into_event() {
        let mut event = AgUiEvent {
            event_type: EVENT_RUN_STARTED.to_string(),
            payload: json!({"thread_id": "t1"}),
        };
        let mut meta = serde_json::Map::new();
        meta.insert("user_id".to_string(), json!("u1"));
        event.inject_metadata(meta);
        assert_eq!(event.payload["_gate_metadata"]["user_id"], "u1");
    }

    // ── Per-tool-call authorization on the AG-UI stream ─────────────────────

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

    fn ctx_from(policy: &str) -> ToolAuthzContext {
        let engine = AuthzEngine::from_records(&[record("p", policy)]).expect("compiles");
        ToolAuthzContext {
            engine: Arc::new(engine),
            principal_id: "alice".to_string(),
            route_id: "route-1".to_string(),
            audit: None,
        }
    }

    fn start(id: &str, name: &str) -> AgUiEvent {
        AgUiEvent {
            event_type: EVENT_TOOL_CALL_START.to_string(),
            payload: json!({"toolCallId": id, "toolCallName": name}),
        }
    }
    fn args(id: &str, delta: &str) -> AgUiEvent {
        AgUiEvent {
            event_type: EVENT_TOOL_CALL_ARGS.to_string(),
            payload: json!({"toolCallId": id, "delta": delta}),
        }
    }
    fn end(id: &str) -> AgUiEvent {
        AgUiEvent {
            event_type: EVENT_TOOL_CALL_END.to_string(),
            payload: json!({"toolCallId": id}),
        }
    }

    fn processor_with(policy: &str) -> AgUiProcessor {
        AgUiProcessor::new(false, vec![]).with_tool_authz(Some(ctx_from(policy)))
    }

    fn deny_all_processor() -> AgUiProcessor {
        AgUiProcessor::new(false, vec![]).with_tool_authz(Some(ToolAuthzContext {
            engine: Arc::new(AuthzEngine::empty()),
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        }))
    }

    fn types(events: &[AgUiEvent]) -> Vec<&str> {
        events.iter().map(|e| e.event_type.as_str()).collect()
    }

    // ── Buffer-until-authorized: allow path holds until END, then flushes ────

    #[test]
    fn allowed_call_holds_until_end_then_flushes_in_order() {
        let proc = processor_with("permit(principal, action, resource);");
        // START is HELD — nothing forwarded yet.
        assert!(
            proc.process_multi(start("c1", "read"), Default::default())
                .is_empty(),
            "allowed START must be held, not forwarded"
        );
        // ARGS are HELD — nothing forwarded.
        assert!(proc
            .process_multi(args("c1", "{\"x\":"), Default::default())
            .is_empty());
        assert!(proc
            .process_multi(args("c1", "1}"), Default::default())
            .is_empty());
        // END authorizes on full args → flush START + coalesced ARGS + END.
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(
            types(&released),
            vec![
                EVENT_TOOL_CALL_START,
                EVENT_TOOL_CALL_ARGS,
                EVENT_TOOL_CALL_END
            ],
            "held call flushes START, coalesced ARGS, END in order"
        );
        // The coalesced ARGS carries the COMPLETE argument string.
        assert_eq!(released[1].payload["delta"], "{\"x\":1}");
        assert_eq!(released[1].payload["toolCallId"], "c1");
    }

    #[test]
    fn allowed_no_arg_call_flushes_start_and_end_without_args_event() {
        let proc = processor_with("permit(principal, action, resource);");
        proc.process_multi(start("c1", "ping"), Default::default());
        let released = proc.process_multi(end("c1"), Default::default());
        // No ARGS were sent → no coalesced ARGS event, just START + END.
        assert_eq!(
            types(&released),
            vec![EVENT_TOOL_CALL_START, EVENT_TOOL_CALL_END]
        );
    }

    // ── Deny at END: nothing of the call reaches the client, only RUN_ERROR ──

    /// Permit by name, but forbid a specific argument. With buffer-until-
    /// authorized the forwarding boundary is END, so this refined form works.
    fn permit_name_forbid_arg() -> &'static str {
        concat!(
            "permit(principal, action, resource);\n",
            r#"forbid(principal, action, resource) when { context.arguments.path == "/etc/passwd" };"#
        )
    }

    #[test]
    fn denied_at_end_emits_run_error_and_forwards_no_args() {
        let proc = processor_with(permit_name_forbid_arg());
        // START allowed by name → held (nothing out).
        assert!(proc
            .process_multi(start("c1", "read_file"), Default::default())
            .is_empty());
        // The forbidden path accumulates across deltas — still held, no leak.
        assert!(proc
            .process_multi(args("c1", "{\"path\":\"/e"), Default::default())
            .is_empty());
        assert!(proc
            .process_multi(args("c1", "tc/passwd\"}"), Default::default())
            .is_empty());
        // END on full args → DENY. Only a single RUN_ERROR; no START/ARGS/END.
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(types(&released), vec![EVENT_RUN_ERROR]);
        assert_eq!(released[0].payload["toolCallId"], "c1");
        // Assert ABSENCE: no tool-call args bytes ever forwarded.
        assert!(
            !released
                .iter()
                .any(|e| e.event_type == EVENT_TOOL_CALL_ARGS),
            "denied call must not forward any ARGS"
        );
    }

    #[test]
    fn allowed_at_end_when_full_args_are_safe() {
        let proc = processor_with(permit_name_forbid_arg());
        proc.process_multi(start("c1", "read_file"), Default::default());
        proc.process_multi(args("c1", "{\"path\":\"/tmp/ok\"}"), Default::default());
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(
            types(&released),
            vec![
                EVENT_TOOL_CALL_START,
                EVENT_TOOL_CALL_ARGS,
                EVENT_TOOL_CALL_END
            ]
        );
    }

    // ── Deny at START (coarse) ───────────────────────────────────────────────

    #[test]
    fn denied_at_start_emits_run_error_and_drops_args_end() {
        let proc = deny_all_processor();
        // START → synthetic RUN_ERROR (the START itself is dropped).
        let released = proc.process_multi(start("c1", "danger"), Default::default());
        assert_eq!(types(&released), vec![EVENT_RUN_ERROR]);
        assert_eq!(released[0].payload["code"], "tool_call_denied");
        assert_eq!(released[0].payload["toolCallId"], "c1");
        // Subsequent ARGS and END for the blocked id are dropped entirely.
        assert!(proc
            .process_multi(args("c1", "{}"), Default::default())
            .is_empty());
        assert!(proc.process_multi(end("c1"), Default::default()).is_empty());
    }

    // ── Fail-closed edge cases ───────────────────────────────────────────────

    #[test]
    fn malformed_start_missing_name_fails_closed() {
        let proc = processor_with("permit(principal, action, resource);");
        let ev = AgUiEvent {
            event_type: EVENT_TOOL_CALL_START.to_string(),
            payload: json!({"toolCallId": "c1"}), // no name
        };
        let released = proc.process_multi(ev, Default::default());
        assert_eq!(
            types(&released),
            vec![EVENT_RUN_ERROR],
            "no name → denied (fail-closed)"
        );
    }

    #[test]
    fn args_and_end_without_start_are_dropped() {
        let proc = processor_with("permit(principal, action, resource);");
        assert!(proc
            .process_multi(args("c9", "{}"), Default::default())
            .is_empty());
        assert!(proc.process_multi(end("c9"), Default::default()).is_empty());
    }

    // ── L2: non-empty unparseable args at END → DENY (not coerced to {}) ─────

    #[test]
    fn unparseable_args_at_end_denies_fail_closed() {
        let proc = processor_with("permit(principal, action, resource);");
        proc.process_multi(start("c1", "read"), Default::default());
        // Non-empty, invalid JSON args.
        proc.process_multi(args("c1", "{not valid json"), Default::default());
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(
            types(&released),
            vec![EVENT_RUN_ERROR],
            "unparseable non-empty args must DENY, not coerce to {{}} and allow"
        );
    }

    #[test]
    fn empty_args_buffer_is_treated_as_no_arg_call() {
        // Distinguishes L2: an EMPTY buffer is a legitimate no-arg call → allow.
        let proc = processor_with("permit(principal, action, resource);");
        assert!(parse_args_buffer("").is_some());
        assert!(parse_args_buffer("   ").is_some());
        // And a whitespace-only ARGS stream still flushes as a no-arg call.
        proc.process_multi(start("c1", "ping"), Default::default());
        proc.process_multi(args("c1", "   "), Default::default());
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(
            types(&released),
            vec![EVENT_TOOL_CALL_START, EVENT_TOOL_CALL_END]
        );
    }

    // ── C1: per-tool-call args byte cap → DENY that call ─────────────────────

    #[test]
    fn oversized_tool_args_denied_without_terminating() {
        let proc = AgUiProcessor::new(false, vec![])
            .with_tool_authz(Some(ctx_from("permit(principal, action, resource);")))
            .with_max_tool_args_bytes(16);
        proc.process_multi(start("c1", "read"), Default::default());
        // A single oversized delta trips the cap → RUN_ERROR for this call.
        let released = proc.process_multi(
            args("c1", "0123456789abcdefghijXYZ-way-over-16-bytes"),
            Default::default(),
        );
        assert_eq!(types(&released), vec![EVENT_RUN_ERROR]);
        // State is gone → a later END for the id is a no-op drop (not a flush).
        assert!(proc.process_multi(end("c1"), Default::default()).is_empty());
    }

    // ── Live-streaming guarantees ────────────────────────────────────────────

    #[test]
    fn text_message_content_streams_live_with_authz_present() {
        let proc = processor_with("permit(principal, action, resource);");
        let msg = AgUiEvent {
            event_type: EVENT_TEXT_MESSAGE_CONTENT.to_string(),
            payload: json!({"delta": "hi"}),
        };
        let released = proc.process_multi(msg, Default::default());
        assert_eq!(
            types(&released),
            vec![EVENT_TEXT_MESSAGE_CONTENT],
            "text content is NEVER buffered — streams live"
        );
    }

    #[test]
    fn non_tool_events_pass_through_with_authz_present() {
        let proc = processor_with("permit(principal, action, resource);");
        for et in [EVENT_RUN_STARTED, EVENT_STATE_SNAPSHOT, EVENT_STEP_STARTED] {
            let ev = AgUiEvent {
                event_type: et.to_string(),
                payload: json!({}),
            };
            assert_eq!(
                proc.process_multi(ev, Default::default()).len(),
                1,
                "{et} must stream live"
            );
        }
    }

    #[test]
    fn without_authz_context_tool_calls_stream_live_unaffected() {
        // No ToolAuthzContext → backward-compatible: tool events pass live,
        // one-for-one, with NO buffering.
        let proc = AgUiProcessor::new(false, vec![]);
        assert_eq!(
            types(&proc.process_multi(start("c1", "anything"), Default::default())),
            vec![EVENT_TOOL_CALL_START],
            "START streams live when no authz configured"
        );
        assert_eq!(
            types(&proc.process_multi(args("c1", "{}"), Default::default())),
            vec![EVENT_TOOL_CALL_ARGS]
        );
        assert_eq!(
            types(&proc.process_multi(end("c1"), Default::default())),
            vec![EVENT_TOOL_CALL_END]
        );
    }

    #[test]
    fn snake_case_field_names_are_accepted() {
        let proc = processor_with("permit(principal, action, resource);");
        let s = AgUiEvent {
            event_type: EVENT_TOOL_CALL_START.to_string(),
            payload: json!({"tool_call_id": "c1", "tool_call_name": "read"}),
        };
        // Held (allowed) → no output; END then flushes, proving the id/name were
        // recognized from snake_case fields.
        assert!(proc.process_multi(s, Default::default()).is_empty());
        let released = proc.process_multi(end("c1"), Default::default());
        assert_eq!(
            types(&released),
            vec![EVENT_TOOL_CALL_START, EVENT_TOOL_CALL_END]
        );
    }
}
