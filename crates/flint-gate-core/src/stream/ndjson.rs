/// NDJSON (newline-delimited JSON) stream processor.
///
/// Splits the upstream response on `\n`, parses each line as a JSON object,
/// runs it through the AG-UI/A2UI event chain, and re-emits filtered events.
use crate::approval::{ApprovalDecision, ApprovalManager};
use crate::config::types::StreamConfig;
use crate::stream::a2ui::{A2UiEvent, A2UiProcessor};
use crate::stream::ag_ui::{AgUiEvent, AgUiProcessor, AgUiTokenCounter};
use crate::stream::{ApprovalHandleParts, StreamMetrics};
use bytes::Bytes;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

/// NDJSON stream processor — newline-delimited JSON variant.
pub struct NdjsonStreamProcessor {
    config: StreamConfig,
    line_buffer: Vec<u8>,
    ag_ui_processor: Option<AgUiProcessor>,
    a2ui_processor: Option<A2UiProcessor>,
    metrics: StreamMetrics,
    token_counter: AgUiTokenCounter,
    started_at: Instant,
    user_scopes: Vec<String>,
    metadata: serde_json::Map<String, serde_json::Value>,
    theme: Option<serde_json::Value>,
    // Optional handle for requesting human-in-the-loop approvals. Kept so the
    // processor can be reconstructed with the same configuration; the active
    // sender is already wired into the AG-UI processor at construction time.
    #[allow(dead_code)]
    approval_handle: Option<ApprovalHandle>,
    // C1: cap (bytes) on a single NDJSON line buffered without a newline.
    max_event_bytes: usize,
}

#[derive(Clone)]
struct ApprovalHandle {
    manager: ApprovalManager,
    decision_tx: UnboundedSender<(String, ApprovalDecision)>,
    ttl_override: Option<std::time::Duration>,
}

impl NdjsonStreamProcessor {
    pub fn new(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
    ) -> Self {
        Self::with_tool_authz(config, user_scopes, metadata, theme, None)
    }

    /// Create a new processor, optionally threading a per-tool-call
    /// authorization context into the AG-UI processor. `None` → identical to
    /// [`Self::new`] (backward-compatible).
    pub fn with_tool_authz(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
        tool_authz: Option<crate::authz::ToolAuthzContext>,
    ) -> Self {
        Self::with_tool_authz_and_approval(config, user_scopes, metadata, theme, tool_authz, None)
    }

    /// Create a new processor with optional tool authz and optional human
    /// approval support.
    pub fn with_tool_authz_and_approval(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
        tool_authz: Option<crate::authz::ToolAuthzContext>,
        approval_handle: Option<ApprovalHandleParts>,
    ) -> Self {
        let max_tool_args_bytes = config
            .ai
            .backpressure
            .max_tool_args_bytes
            .unwrap_or(crate::stream::DEFAULT_MAX_TOOL_ARGS_BYTES);

        let approval_handle =
            approval_handle.map(|(manager, decision_tx, ttl_override)| ApprovalHandle {
                manager,
                decision_tx,
                ttl_override,
            });

        let ag_ui_processor = if config.ai.ag_ui.enabled {
            let mut proc = AgUiProcessor::new(
                config.ai.ag_ui.validate_events,
                config.ai.ag_ui.allowed_events.clone(),
            )
            .with_tool_authz(tool_authz.clone())
            .with_max_tool_args_bytes(max_tool_args_bytes);
            if let Some(handle) = approval_handle.clone() {
                proc = proc.with_approval_handle(handle.manager, handle.decision_tx, handle.ttl_override);
            }
            Some(proc)
        } else {
            None
        };

        let a2ui_processor = if config.ai.a2ui.enabled {
            let mut proc = A2UiProcessor::new(config.ai.a2ui.allowed_intents.clone())
                .with_tool_authz(tool_authz.clone());
            if let Some(handle) = approval_handle.clone() {
                proc = proc.with_approval_handle(handle.manager, handle.decision_tx, handle.ttl_override);
            }
            Some(proc)
        } else {
            None
        };

        let max_event_bytes = config
            .ai
            .backpressure
            .max_event_bytes
            .unwrap_or(crate::stream::DEFAULT_MAX_EVENT_BYTES);

        Self {
            config,
            line_buffer: Vec::new(),
            ag_ui_processor,
            a2ui_processor,
            metrics: StreamMetrics::default(),
            token_counter: AgUiTokenCounter::default(),
            started_at: Instant::now(),
            max_event_bytes,
            user_scopes,
            metadata,
            theme,
            approval_handle,
        }
    }

    /// Ids of approvals currently pending in the AG-UI or A2UI processor.
    pub fn pending_approvals(&self) -> Vec<String> {
        let mut ids = self
            .ag_ui_processor
            .as_ref()
            .map(|p| p.pending_approval_ids())
            .unwrap_or_default();
        ids.extend(
            self.a2ui_processor
                .as_ref()
                .map(|p| p.pending_approval_ids())
                .unwrap_or_default(),
        );
        ids
    }

    /// Earliest monotonic deadline across all pending approvals.
    pub fn earliest_pending_deadline(&self) -> Option<std::time::Instant> {
        [
            self.ag_ui_processor
                .as_ref()
                .and_then(|p| p.earliest_pending_deadline()),
            self.a2ui_processor
                .as_ref()
                .and_then(|p| p.earliest_pending_deadline()),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Resolve a pending approval, returning the NDJSON bytes to forward.
    pub fn resolve_approval(
        &mut self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Option<Bytes> {
        // Prefer AG-UI; if it owns the id, it releases held tool-call frames.
        if let Some(ag_ui) = self.ag_ui_processor.as_mut() {
            if ag_ui
                .pending_approval_ids()
                .contains(&approval_id.to_string())
            {
                let released = ag_ui.resolve_approval(approval_id, decision);
                if released.is_empty() {
                    return None;
                }
                self.metrics.passed_events += released.len() as u64;
                let joined = released
                    .iter()
                    .map(AgUiEvent::to_json)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(Bytes::from(joined + "\n"));
            }
        }

        // Otherwise resolve against the A2UI processor.
        let a2ui = self.a2ui_processor.as_mut()?;
        let event = a2ui.resolve_approval(approval_id, decision)?;
        self.metrics.passed_events += 1;
        Some(Bytes::from(event.to_json() + "\n"))
    }

    /// Process a complete NDJSON line.
    fn process_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        self.metrics.total_events += 1;

        // Try AG-UI processing. `process_multi` returns 0..N events: 0 when
        // dropped or HELD (buffered tool call), N when a `TOOL_CALL_END`
        // releases a held call. Each released event is its own NDJSON line.
        if let Some(ref ag_ui_proc) = self.ag_ui_processor {
            if let Some(event) = AgUiEvent::from_json(line) {
                self.token_counter.count_event(&event);
                let meta = self.metadata.clone();
                let released = ag_ui_proc.process_multi(event, meta);
                if released.is_empty() {
                    self.metrics.dropped_events += 1;
                    return None;
                }
                self.metrics.passed_events += 1;
                let joined = released
                    .iter()
                    .map(AgUiEvent::to_json)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(joined);
            }
        }

        // Try A2UI processing
        if let Some(ref a2ui_proc) = self.a2ui_processor {
            if let Some(event) = A2UiEvent::from_json(line) {
                match a2ui_proc.process(event, &self.user_scopes, self.theme.clone()) {
                    None => {
                        // Dropped, OR held pending human approval.
                        if !a2ui_proc.pending_approval_ids().is_empty() {
                            if let Some(req_event) = a2ui_proc.approval_request_event() {
                                self.metrics.passed_events += 1;
                                return Some(req_event.to_json());
                            }
                        }
                        self.metrics.dropped_events += 1;
                        return None;
                    }
                    Some(processed) => {
                        self.metrics.passed_events += 1;
                        return Some(processed.to_json());
                    }
                }
            }
        }

        // No AI processing — pass through as-is
        self.metrics.passed_events += 1;
        Some(line.to_string())
    }
}

impl crate::stream::StreamProcessor for NdjsonStreamProcessor {
    fn process_chunk(&mut self, chunk: &[u8]) -> Option<Bytes> {
        // Backpressure: duration
        if let Some(max_secs) = self.config.ai.backpressure.max_stream_duration_seconds {
            if self.started_at.elapsed().as_secs() > max_secs {
                self.metrics.terminated_by_limit = true;
                tracing::warn!("NDJSON stream terminated: duration limit exceeded");
                return None;
            }
        }

        // Backpressure: event count
        if let Some(max_events) = self.config.ai.backpressure.max_events {
            if self.metrics.total_events >= max_events {
                self.metrics.terminated_by_limit = true;
                tracing::warn!("NDJSON stream terminated: event count limit exceeded");
                return None;
            }
        }

        let mut output = Vec::with_capacity(chunk.len());
        let mut pos = 0;

        while pos < chunk.len() {
            match chunk[pos..].iter().position(|&b| b == b'\n') {
                Some(newline_pos) => {
                    // C1: a line already over the cap before its newline is DoS.
                    if self.line_buffer.len().saturating_add(newline_pos) > self.max_event_bytes {
                        self.metrics.terminated_by_limit = true;
                        tracing::warn!(
                            cap = self.max_event_bytes,
                            "NDJSON stream terminated: line exceeded byte cap"
                        );
                        return None;
                    }
                    self.line_buffer
                        .extend_from_slice(&chunk[pos..pos + newline_pos]);
                    let line = std::mem::take(&mut self.line_buffer);
                    let line_str = String::from_utf8_lossy(&line);

                    if let Some(processed) = self.process_line(&line_str) {
                        output.extend_from_slice(processed.as_bytes());
                        output.push(b'\n');
                    }

                    pos += newline_pos + 1;
                }
                None => {
                    // C1: cap the partial line so an upstream that never emits a
                    // newline cannot grow the buffer without bound.
                    let remaining = chunk.len() - pos;
                    if self.line_buffer.len().saturating_add(remaining) > self.max_event_bytes {
                        self.metrics.terminated_by_limit = true;
                        tracing::warn!(
                            cap = self.max_event_bytes,
                            "NDJSON stream terminated: unbounded partial line exceeded byte cap"
                        );
                        return None;
                    }
                    self.line_buffer.extend_from_slice(&chunk[pos..]);
                    break;
                }
            }
        }

        self.metrics.duration_ms = self.started_at.elapsed().as_millis() as u64;
        self.metrics.estimated_tokens = self.token_counter.estimated_tokens();

        Some(Bytes::from(output))
    }

    fn metrics(&self) -> &StreamMetrics {
        &self.metrics
    }

    fn terminated_by_limit(&self) -> bool {
        self.metrics.terminated_by_limit
    }

    fn termination_payload(&self) -> Vec<u8> {
        b"{\"error\":\"stream limit exceeded\"}\n".to_vec()
    }

    fn pending_approvals(&self) -> Vec<String> {
        self.pending_approvals()
    }

    fn resolve_approval(
        &mut self,
        approval_id: &str,
        decision: crate::approval::ApprovalDecision,
    ) -> Option<Bytes> {
        self.resolve_approval(approval_id, decision)
    }

    fn earliest_pending_deadline(&self) -> Option<std::time::Instant> {
        self.earliest_pending_deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalDecision, ApprovalManager};
    use crate::authz::{AuthzEngine, PolicyRecord, ToolAuthzContext};
    use crate::config::types::{A2UiConfig, AgUiConfig, AiStreamConfig, StreamConfig};
    use crate::stream::StreamProcessor;
    use std::sync::Arc;

    fn passthrough_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "ndjson".to_string(),
            ai: AiStreamConfig::default(),
        }
    }

    #[test]
    fn passthrough_ndjson_lines() {
        let mut proc =
            NdjsonStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        let input = b"{\"type\":\"message\",\"data\":\"hello\"}\n";
        let output = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&output).unwrap();
        assert!(s.contains("hello"));
    }

    #[test]
    fn multiple_ndjson_lines() {
        let mut proc =
            NdjsonStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        proc.process_chunk(b"{\"i\":1}\n");
        proc.process_chunk(b"{\"i\":2}\n");
        assert_eq!(proc.metrics().total_events, 2);
        assert_eq!(proc.metrics().passed_events, 2);
    }

    #[test]
    fn buffers_partial_lines() {
        let mut proc =
            NdjsonStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        // Send partial line (no newline)
        let output1 = proc.process_chunk(b"{\"partial\":").unwrap();
        assert!(output1.is_empty());
        // Complete the line
        let output2 = proc.process_chunk(b"true}\n").unwrap();
        let s = std::str::from_utf8(&output2).unwrap();
        assert!(s.contains("partial"));
    }

    // ── add-hitl-approval: NDJSON A2UI/AG-UI pause → approve/deny ───────────

    fn ndjson_ag_ui_enabled_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "ndjson".to_string(),
            ai: AiStreamConfig {
                ag_ui: AgUiConfig {
                    enabled: true,
                    validate_events: false,
                    allowed_events: vec![],
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    fn ndjson_a2ui_enabled_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "ndjson".to_string(),
            ai: AiStreamConfig {
                a2ui: A2UiConfig {
                    enabled: true,
                    allowed_intents: vec![],
                    theme: None,
                },
                ..Default::default()
            },
        }
    }

    fn ctx(engine: AuthzEngine) -> ToolAuthzContext {
        ToolAuthzContext {
            engine: Arc::new(engine),
            principal_kind: crate::authz::PrincipalKind::User,
            revoked: false,
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
            audit: None,
        }
    }

    fn require_approval_at_end_engine() -> AuthzEngine {
        AuthzEngine::from_records(&[
            PolicyRecord {
                id: "allow_empty".to_string(),
                policy_text:
                    r#"permit(principal, action, resource) when { context.arguments == {} };"#
                        .to_string(),
                schema_json: None,
                entities_json: None,
            },
            PolicyRecord {
                id: "require_args".to_string(),
                policy_text: r#"@require_approval("non-empty arguments")
                    permit(principal, action, resource) when { context.arguments != {} };"#
                    .to_string(),
                schema_json: None,
                entities_json: None,
            },
        ])
        .expect("compiles")
    }

    #[test]
    fn ndjson_a2ui_require_approval_emits_gate_approval_request() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(String, ApprovalDecision)>();
        let mut proc = NdjsonStreamProcessor::with_tool_authz_and_approval(
            ndjson_a2ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(require_approval_at_end_engine())),
            Some((manager, tx, None)),
        );

        let out = proc
            .process_chunk(
                br#"{"intent":"invoke_tool","tool_name":"danger","arguments":{"x":1}}
"#,
            )
            .unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("gate:approval_request"),
            "A2UI tool call must emit approval request: {s}"
        );
        assert!(
            !s.contains(r#""tool_name":"danger""#),
            "original event must be held: {s}"
        );
        assert_eq!(proc.pending_approvals().len(), 1);
    }

    #[test]
    fn ndjson_a2ui_approve_releases_held_event() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(String, ApprovalDecision)>();
        let mut proc = NdjsonStreamProcessor::with_tool_authz_and_approval(
            ndjson_a2ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(require_approval_at_end_engine())),
            Some((manager, tx, None)),
        );

        proc.process_chunk(
            br#"{"intent":"invoke_tool","tool_name":"danger","arguments":{"x":1}}
"#,
        )
        .unwrap();
        let id = proc.pending_approvals()[0].clone();

        let released = proc
            .resolve_approval(&id, ApprovalDecision::Approve)
            .unwrap();
        let s = std::str::from_utf8(&released).unwrap();
        assert!(
            s.contains("invoke_tool"),
            "approved A2UI event must be released: {s}"
        );
        assert!(proc.pending_approvals().is_empty());
    }

    #[test]
    fn ndjson_a2ui_deny_drops_held_event() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(String, ApprovalDecision)>();
        let mut proc = NdjsonStreamProcessor::with_tool_authz_and_approval(
            ndjson_a2ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(require_approval_at_end_engine())),
            Some((manager, tx, None)),
        );

        proc.process_chunk(
            br#"{"intent":"invoke_tool","tool_name":"danger","arguments":{"x":1}}
"#,
        )
        .unwrap();
        let id = proc.pending_approvals()[0].clone();

        assert!(
            proc.resolve_approval(&id, ApprovalDecision::Deny).is_none(),
            "denied A2UI event must be dropped"
        );
        assert!(proc.pending_approvals().is_empty());
    }

    #[test]
    fn ndjson_ag_ui_approve_at_end_releases_full_call() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(String, ApprovalDecision)>();
        let mut proc = NdjsonStreamProcessor::with_tool_authz_and_approval(
            ndjson_ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(require_approval_at_end_engine())),
            Some((manager, tx, None)),
        );

        proc.process_chunk(
            b"{\"type\":\"TOOL_CALL_START\",\"toolCallId\":\"c1\",\"toolCallName\":\"x\"}\n",
        )
        .unwrap();
        proc.process_chunk(
            b"{\"type\":\"TOOL_CALL_ARGS\",\"toolCallId\":\"c1\",\"delta\":\"{\\\"x\\\":1}\"}\n",
        )
        .unwrap();
        proc.process_chunk(b"{\"type\":\"TOOL_CALL_END\",\"toolCallId\":\"c1\"}\n")
            .unwrap();
        let id = proc.pending_approvals()[0].clone();

        let released = proc
            .resolve_approval(&id, ApprovalDecision::Approve)
            .unwrap();
        let s = std::str::from_utf8(&released).unwrap();
        assert!(s.contains("TOOL_CALL_START"), "START released: {s}");
        assert!(s.contains("TOOL_CALL_END"), "END released: {s}");
        assert!(proc.pending_approvals().is_empty());
    }

    #[test]
    fn ndjson_ag_ui_deny_at_end_emits_run_error() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<(String, ApprovalDecision)>();
        let mut proc = NdjsonStreamProcessor::with_tool_authz_and_approval(
            ndjson_ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(require_approval_at_end_engine())),
            Some((manager, tx, None)),
        );

        proc.process_chunk(
            b"{\"type\":\"TOOL_CALL_START\",\"toolCallId\":\"c1\",\"toolCallName\":\"x\"}\n",
        )
        .unwrap();
        proc.process_chunk(
            b"{\"type\":\"TOOL_CALL_ARGS\",\"toolCallId\":\"c1\",\"delta\":\"{\\\"x\\\":1}\"}\n",
        )
        .unwrap();
        proc.process_chunk(b"{\"type\":\"TOOL_CALL_END\",\"toolCallId\":\"c1\"}\n")
            .unwrap();
        let id = proc.pending_approvals()[0].clone();

        let released = proc.resolve_approval(&id, ApprovalDecision::Deny).unwrap();
        let s = std::str::from_utf8(&released).unwrap();
        assert!(
            s.contains("RUN_ERROR"),
            "denied AG-UI call must emit RUN_ERROR: {s}"
        );
        assert!(proc.pending_approvals().is_empty());
    }
}
