/// SSE stream processor — the core streaming engine.
///
/// Buffers partial SSE lines, assembles complete events, dispatches through
/// AG-UI/A2UI processors, tracks metrics, and enforces backpressure limits.
use crate::config::types::StreamConfig;
use crate::stream::a2ui::{A2UiEvent, A2UiProcessor};
use crate::stream::ag_ui::{AgUiEvent, AgUiProcessor, AgUiTokenCounter};
use bytes::Bytes;
use std::time::Instant;

/// Metrics collected during SSE stream processing.
#[derive(Debug, Clone, Default)]
pub struct StreamMetrics {
    /// Total SSE events processed.
    pub total_events: u64,
    /// Events that were passed through (not filtered).
    pub passed_events: u64,
    /// Events that were dropped by AG-UI/A2UI filters.
    pub dropped_events: u64,
    /// Estimated tokens from TEXT_MESSAGE_CONTENT deltas.
    pub estimated_tokens: u64,
    /// Stream duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the stream was terminated by a backpressure limit.
    pub terminated_by_limit: bool,
}

/// Reason a stream was terminated early.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TerminationReason {
    DurationLimit,
    EventLimit,
}

/// Outcome of processing one SSE line.
enum LineOutcome {
    /// Forward this text to the client.
    Forward(String),
    /// Buffered or filtered — emit nothing for this line.
    Drop,
    /// A byte cap was exceeded — terminate the stream (C1 DoS guard).
    Terminate,
}

/// The core SSE stream processor.
pub struct SseStreamProcessor {
    config: StreamConfig,
    // Byte buffer for accumulating partial SSE lines
    line_buffer: Vec<u8>,
    // Current SSE event fields being assembled
    current_event_data: Vec<String>,
    current_event_type: Option<String>,
    // Sub-processors
    ag_ui_processor: Option<AgUiProcessor>,
    a2ui_processor: Option<A2UiProcessor>,
    // Metrics
    metrics: StreamMetrics,
    token_counter: AgUiTokenCounter,
    started_at: Instant,
    // User scopes for A2UI filtering
    user_scopes: Vec<String>,
    // AG-UI metadata to inject into each event's _gate_metadata
    metadata: serde_json::Map<String, serde_json::Value>,
    // A2UI theme to inject into render_component payloads
    theme: Option<serde_json::Value>,
    // Per-tool-call authorization context (also used to filter tools/list
    // responses that arrive wrapped in an SSE data frame). `None` → unaffected.
    tool_authz: Option<crate::authz::ToolAuthzContext>,
    // C1: cap (bytes) on a single assembled event's `data:` payload and on the
    // raw line buffer. Exceeding it terminates the stream (fail-closed).
    max_event_bytes: usize,
    // Running byte total of the current event's buffered `data:` lines.
    current_event_bytes: usize,
}

impl SseStreamProcessor {
    /// Create a new processor from route stream config.
    pub fn new(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
    ) -> Self {
        Self::with_tool_authz(config, user_scopes, metadata, theme, None)
    }

    /// Create a new processor, optionally threading a per-tool-call
    /// authorization context into the AG-UI processor. When `tool_authz` is
    /// `None`, behavior is identical to [`Self::new`] (backward-compatible).
    pub fn with_tool_authz(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
        tool_authz: Option<crate::authz::ToolAuthzContext>,
    ) -> Self {
        let max_event_bytes = config
            .ai
            .backpressure
            .max_event_bytes
            .unwrap_or(crate::stream::DEFAULT_MAX_EVENT_BYTES);
        let max_tool_args_bytes = config
            .ai
            .backpressure
            .max_tool_args_bytes
            .unwrap_or(crate::stream::DEFAULT_MAX_TOOL_ARGS_BYTES);

        let ag_ui_processor = if config.ai.ag_ui.enabled {
            Some(
                AgUiProcessor::new(
                    config.ai.ag_ui.validate_events,
                    config.ai.ag_ui.allowed_events.clone(),
                )
                .with_tool_authz(tool_authz.clone())
                .with_max_tool_args_bytes(max_tool_args_bytes),
            )
        } else {
            None
        };

        let a2ui_processor = if config.ai.a2ui.enabled {
            Some(
                A2UiProcessor::new(config.ai.a2ui.allowed_intents.clone())
                    .with_tool_authz(tool_authz.clone()),
            )
        } else {
            None
        };

        Self {
            config,
            line_buffer: Vec::new(),
            current_event_data: Vec::new(),
            current_event_type: None,
            ag_ui_processor,
            a2ui_processor,
            metrics: StreamMetrics::default(),
            token_counter: AgUiTokenCounter::default(),
            started_at: Instant::now(),
            tool_authz,
            user_scopes,
            metadata,
            theme,
            max_event_bytes,
            current_event_bytes: 0,
        }
    }

    /// Process a raw chunk of bytes from the upstream SSE stream.
    ///
    /// Returns the filtered/processed bytes to forward to the client.
    /// Returns `None` if a backpressure limit has been hit (terminate stream).
    pub fn process_chunk(&mut self, chunk: &[u8]) -> Option<Bytes> {
        // Check duration limit
        if let Some(max_secs) = self.config.ai.backpressure.max_stream_duration_seconds {
            if self.started_at.elapsed().as_secs() > max_secs {
                self.metrics.terminated_by_limit = true;
                tracing::warn!("SSE stream terminated: duration limit exceeded");
                return None;
            }
        }

        // Check event count limit
        if let Some(max_events) = self.config.ai.backpressure.max_events {
            if self.metrics.total_events >= max_events {
                self.metrics.terminated_by_limit = true;
                tracing::warn!("SSE stream terminated: event count limit exceeded");
                return None;
            }
        }

        let mut output = Vec::with_capacity(chunk.len());
        let mut pos = 0;

        while pos < chunk.len() {
            match memchr::memchr(b'\n', &chunk[pos..]) {
                Some(newline_pos) => {
                    // Accumulate into line_buffer + process the complete line.
                    // C1: a single line that already overruns the cap (before we
                    // even see its newline) is a DoS — terminate the stream.
                    if self.line_buffer.len().saturating_add(newline_pos) > self.max_event_bytes {
                        self.metrics.terminated_by_limit = true;
                        tracing::warn!(
                            cap = self.max_event_bytes,
                            "SSE stream terminated: line exceeded byte cap"
                        );
                        return None;
                    }
                    self.line_buffer
                        .extend_from_slice(&chunk[pos..pos + newline_pos]);
                    let line = std::mem::take(&mut self.line_buffer);
                    let line_str = String::from_utf8_lossy(&line);

                    match self.process_line(&line_str) {
                        LineOutcome::Forward(processed) => {
                            output.extend_from_slice(processed.as_bytes());
                            output.push(b'\n');
                        }
                        LineOutcome::Drop => {}
                        LineOutcome::Terminate => {
                            self.metrics.terminated_by_limit = true;
                            return None;
                        }
                    }

                    pos += newline_pos + 1;
                }
                None => {
                    // No newline in this chunk — buffer it. C1: cap the partial
                    // line buffer so an upstream that never emits `\n` cannot
                    // grow it without bound.
                    let remaining = chunk.len() - pos;
                    if self.line_buffer.len().saturating_add(remaining) > self.max_event_bytes {
                        self.metrics.terminated_by_limit = true;
                        tracing::warn!(
                            cap = self.max_event_bytes,
                            "SSE stream terminated: unbounded partial line exceeded byte cap"
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

    /// Process a single SSE line: forward it, drop it (buffered), or terminate.
    fn process_line(&mut self, line: &str) -> LineOutcome {
        let line = line.trim_end_matches('\r');

        // Empty line = end of SSE event
        if line.is_empty() {
            self.current_event_bytes = 0; // reset per-event byte total
            return match self.flush_event() {
                Some(s) => LineOutcome::Forward(format!("{s}\n")),
                None => LineOutcome::Drop,
            };
        }

        // SSE field: `data: ...`
        //
        // Buffer the payload and emit NOTHING now: the complete event is
        // reconstructed and forwarded (or dropped/filtered) in `flush_event` at
        // the blank line. Emitting the raw `data:` line here would leak the
        // original payload even when the event is later dropped — defeating the
        // AG-UI/A2UI/tool-authz filters. (This is the seam that makes a `Deny`
        // actually suppress the tool-call event.)
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            // C1: cap the assembled event payload across multi-line `data:`.
            self.current_event_bytes = self.current_event_bytes.saturating_add(data.len());
            if self.current_event_bytes > self.max_event_bytes {
                tracing::warn!(
                    cap = self.max_event_bytes,
                    "SSE stream terminated: event data exceeded byte cap"
                );
                return LineOutcome::Terminate;
            }
            self.current_event_data.push(data.to_string());
            return LineOutcome::Drop;
        }

        // SSE field: `event: ...` — buffer for the flush; do not emit yet, so
        // the event line stays attached to its (possibly dropped) data.
        if let Some(event_type) = line.strip_prefix("event:") {
            self.current_event_type = Some(event_type.trim().to_string());
            return LineOutcome::Drop;
        }

        // SSE comments `:`
        if line.starts_with(':') {
            return LineOutcome::Forward(line.to_string());
        }

        // Other fields (id:, retry:) — pass through
        LineOutcome::Forward(line.to_string())
    }

    /// Called when we hit a blank line (end of SSE event).
    ///
    /// If no AG-UI/A2UI processing, returns an empty string (event separator).
    /// Otherwise tries to parse and filter the data.
    fn flush_event(&mut self) -> Option<String> {
        let data_lines = std::mem::take(&mut self.current_event_data);
        let event_type = self.current_event_type.take();

        if data_lines.is_empty() {
            return Some(String::new()); // empty SSE separator
        }

        let data_str = data_lines.join("\n");

        // Re-emit a buffered `event:` line ahead of the (possibly rewritten)
        // `data:` line. `None` when the upstream sent no explicit event field.
        let prefix = |body: String| -> String {
            match &event_type {
                Some(et) => format!("event: {et}\ndata: {body}"),
                None => format!("data: {body}"),
            }
        };

        // [DONE] sentinel — always pass through
        if data_str.trim() == "[DONE]" {
            self.metrics.total_events += 1;
            self.metrics.passed_events += 1;
            return Some(prefix(data_str));
        }

        self.metrics.total_events += 1;

        // Try AG-UI processing. `process_multi` returns 0..N events: 0 when the
        // event is dropped or HELD (a buffered tool call awaiting END), 1 for a
        // normal event, and N when a `TOOL_CALL_END` releases a held call
        // (START + coalesced ARGS + END). Each released event becomes its own
        // SSE frame; when none are released the whole event is dropped.
        if let Some(ag_ui_proc) = &self.ag_ui_processor {
            if let Some(event) = AgUiEvent::from_json(&data_str) {
                self.token_counter.count_event(&event);

                let meta = self.metadata.clone();
                let released = ag_ui_proc.process_multi(event, meta);
                if released.is_empty() {
                    self.metrics.dropped_events += 1;
                    return None; // dropped or held — nothing to forward now
                }
                self.metrics.passed_events += 1;
                // Frame each released event as `[event: T\n]data: J`, joined by
                // the blank-line SSE separator. The buffered upstream `event:`
                // line (if any) belonged to the END event and is applied once,
                // to the last released frame.
                let last = released.len() - 1;
                let framed: Vec<String> = released
                    .iter()
                    .enumerate()
                    .map(|(i, ev)| {
                        let json = ev.to_json();
                        if i == last {
                            prefix(json)
                        } else {
                            format!("data: {json}")
                        }
                    })
                    .collect();
                return Some(framed.join("\n\n"));
            }
        }

        // Try A2UI processing
        if let Some(a2ui_proc) = &self.a2ui_processor {
            if let Some(event) = A2UiEvent::from_json(&data_str) {
                match a2ui_proc.process(event, &self.user_scopes, self.theme.clone()) {
                    None => {
                        self.metrics.dropped_events += 1;
                        return None;
                    }
                    Some(processed) => {
                        self.metrics.passed_events += 1;
                        let json = processed.to_json();
                        return Some(prefix(json));
                    }
                }
            }
        }

        // Task 3 (SSE seam): an MCP `tools/list` response can arrive wrapped in
        // an SSE `data:` frame (Streamable HTTP transport). It is a JSON-RPC
        // object with no AG-UI `type` / A2UI `intent`, so it fell through both
        // filters above. When tool authz is configured, strip denied tools from
        // the listing before forwarding. Non-listing / non-JSON frames are
        // untouched (the helper returns `None`).
        if let Some(ctx) = &self.tool_authz {
            if let Some(filtered) = crate::authz::filter_list_tools_body(
                data_str.as_bytes(),
                &ctx.engine,
                &ctx.principal_id,
                &ctx.route_id,
            ) {
                self.metrics.passed_events += 1;
                let json = String::from_utf8_lossy(&filtered).into_owned();
                return Some(prefix(json));
            }
        }

        // No AI processing or couldn't parse — pass through as-is
        self.metrics.passed_events += 1;
        Some(prefix(data_str))
    }

    /// Return a snapshot of the current stream metrics.
    pub fn metrics(&self) -> &StreamMetrics {
        &self.metrics
    }

    /// Consume the processor and return final metrics.
    #[allow(dead_code)]
    pub fn finish(mut self) -> StreamMetrics {
        self.metrics.duration_ms = self.started_at.elapsed().as_millis() as u64;
        self.metrics.estimated_tokens = self.token_counter.estimated_tokens();
        self.metrics
    }
}

// Use memchr for fast newline scanning
mod memchr {
    pub fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
        haystack.iter().position(|&b| b == needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{AgUiConfig, AiStreamConfig, StreamConfig};

    fn passthrough_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "sse".to_string(),
            ai: AiStreamConfig::default(),
        }
    }

    #[allow(dead_code)] // test fixture retained for AG-UI event-filter cases
    fn ag_ui_config(allowed: Vec<&str>) -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "sse".to_string(),
            ai: AiStreamConfig {
                ag_ui: AgUiConfig {
                    enabled: true,
                    validate_events: true,
                    allowed_events: allowed.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    #[test]
    fn passthrough_simple_event() {
        let mut proc =
            SseStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        let input = b"data: hello\n\n";
        let output = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&output).unwrap();
        assert!(s.contains("data: hello"));
    }

    #[test]
    fn passes_done_sentinel() {
        let mut proc =
            SseStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        let input = b"data: [DONE]\n\n";
        let output = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&output).unwrap();
        assert!(s.contains("[DONE]"));
    }

    #[test]
    fn metrics_accumulate() {
        let mut proc =
            SseStreamProcessor::new(passthrough_config(), vec![], serde_json::Map::new(), None);
        proc.process_chunk(b"data: first\n\n");
        proc.process_chunk(b"data: second\n\n");
        assert_eq!(proc.metrics().total_events, 2);
        assert_eq!(proc.metrics().passed_events, 2);
    }

    // ── Per-tool-call authz wired through the SSE processor ─────────────────

    use crate::authz::{AuthzEngine, PolicyRecord, ToolAuthzContext};
    use std::sync::Arc;

    fn ag_ui_enabled_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "sse".to_string(),
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

    fn ctx(engine: AuthzEngine) -> ToolAuthzContext {
        ToolAuthzContext {
            engine: Arc::new(engine),
            principal_id: "alice".to_string(),
            route_id: "r1".to_string(),
        }
    }

    fn permit_all_engine() -> AuthzEngine {
        AuthzEngine::from_records(&[PolicyRecord {
            id: "p".to_string(),
            policy_text: "permit(principal, action, resource);".to_string(),
            schema_json: None,
            entities_json: None,
        }])
        .expect("compiles")
    }

    #[test]
    fn sse_denied_tool_call_start_emits_run_error_no_start_leaks() {
        let mut proc = SseStreamProcessor::with_tool_authz(
            ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(AuthzEngine::empty())), // deny-all
        );
        let input =
            b"data: {\"type\":\"TOOL_CALL_START\",\"toolCallId\":\"c1\",\"toolCallName\":\"x\"}\n\n";
        let out = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("RUN_ERROR"),
            "denied START becomes a RUN_ERROR: {s}"
        );
        assert!(
            !s.contains("TOOL_CALL_START"),
            "original START must not pass"
        );
    }

    #[test]
    fn sse_allowed_tool_call_holds_then_flushes_at_end() {
        let mut proc = SseStreamProcessor::with_tool_authz(
            ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(permit_all_engine())),
        );
        // START alone is HELD → no tool-call bytes forwarded yet.
        let held = proc
            .process_chunk(
                b"data: {\"type\":\"TOOL_CALL_START\",\"toolCallId\":\"c1\",\"toolCallName\":\"x\"}\n\n",
            )
            .unwrap();
        assert!(
            !std::str::from_utf8(&held)
                .unwrap()
                .contains("TOOL_CALL_START"),
            "allowed START must be held, not streamed live"
        );
        // ARGS held too.
        let held2 = proc
            .process_chunk(
                b"data: {\"type\":\"TOOL_CALL_ARGS\",\"toolCallId\":\"c1\",\"delta\":\"{}\"}\n\n",
            )
            .unwrap();
        assert!(std::str::from_utf8(&held2).unwrap().trim().is_empty());
        // END flushes the whole call.
        let out = proc
            .process_chunk(b"data: {\"type\":\"TOOL_CALL_END\",\"toolCallId\":\"c1\"}\n\n")
            .unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("TOOL_CALL_START"), "START flushed at END: {s}");
        assert!(s.contains("TOOL_CALL_END"), "END flushed: {s}");
    }

    #[test]
    fn sse_text_message_content_streams_live() {
        // TEXT_MESSAGE_CONTENT must NOT be buffered even with authz present.
        let mut proc = SseStreamProcessor::with_tool_authz(
            ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(permit_all_engine())),
        );
        let out = proc
            .process_chunk(b"data: {\"type\":\"TEXT_MESSAGE_CONTENT\",\"delta\":\"live\"}\n\n")
            .unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("TEXT_MESSAGE_CONTENT"), "text streams live: {s}");
        assert!(s.contains("live"));
    }

    #[test]
    fn sse_event_data_over_cap_terminates_stream() {
        // C1: a single event whose assembled data exceeds the cap terminates.
        let mut cfg = ag_ui_enabled_config();
        cfg.ai.backpressure.max_event_bytes = Some(32);
        let mut proc = SseStreamProcessor::new(cfg, vec![], serde_json::Map::new(), None);
        let big = "x".repeat(200);
        let input = format!("data: {big}\n\n");
        let out = proc.process_chunk(input.as_bytes());
        assert!(out.is_none(), "oversized event terminates the stream");
        assert!(proc.metrics().terminated_by_limit);
    }

    #[test]
    fn sse_unbounded_partial_line_over_cap_terminates_stream() {
        // C1: a never-terminated line (no newline) cannot grow past the cap.
        let mut cfg = ag_ui_enabled_config();
        cfg.ai.backpressure.max_event_bytes = Some(32);
        let mut proc = SseStreamProcessor::new(cfg, vec![], serde_json::Map::new(), None);
        let input = "x".repeat(200); // no newline at all
        let out = proc.process_chunk(input.as_bytes());
        assert!(
            out.is_none(),
            "unbounded partial line terminates the stream"
        );
        assert!(proc.metrics().terminated_by_limit);
    }

    #[test]
    fn sse_filters_tools_list_in_data_frame() {
        // Permit only `safe_tool`; a tools/list JSON-RPC frame is filtered.
        let engine = AuthzEngine::from_records(&[PolicyRecord {
            id: "scoped".to_string(),
            policy_text: r#"permit(principal, action, resource == Route::"safe_tool");"#
                .to_string(),
            schema_json: None,
            entities_json: None,
        }])
        .expect("compiles");
        let mut proc = SseStreamProcessor::with_tool_authz(
            ag_ui_enabled_config(),
            vec![],
            serde_json::Map::new(),
            None,
            Some(ctx(engine)),
        );
        let input = b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[{\"name\":\"safe_tool\"},{\"name\":\"danger_tool\"}]}}\n\n";
        let out = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("safe_tool"), "allowed tool remains: {s}");
        assert!(!s.contains("danger_tool"), "denied tool filtered out: {s}");
    }

    #[test]
    fn sse_without_authz_context_passes_tool_calls_untouched() {
        // Backward-compat: no context → tool-call events pass unchanged.
        let mut proc =
            SseStreamProcessor::new(ag_ui_enabled_config(), vec![], serde_json::Map::new(), None);
        let input =
            b"data: {\"type\":\"TOOL_CALL_START\",\"toolCallId\":\"c1\",\"toolCallName\":\"x\"}\n\n";
        let out = proc.process_chunk(input).unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("TOOL_CALL_START"),
            "unaffected without authz: {s}"
        );
    }
}
