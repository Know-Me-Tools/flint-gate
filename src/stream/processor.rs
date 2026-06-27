/// SSE stream processor — the core streaming engine.
///
/// Buffers partial SSE lines, assembles complete events, dispatches through
/// AG-UI/A2UI processors, tracks metrics, and enforces backpressure limits.
use crate::config::types::StreamConfig;
use crate::stream::ag_ui::{AgUiEvent, AgUiProcessor, AgUiTokenCounter};
use crate::stream::a2ui::{A2UiEvent, A2UiProcessor};
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
}

impl SseStreamProcessor {
    /// Create a new processor from route stream config.
    pub fn new(
        config: StreamConfig,
        user_scopes: Vec<String>,
        metadata: serde_json::Map<String, serde_json::Value>,
        theme: Option<serde_json::Value>,
    ) -> Self {
        let ag_ui_processor = if config.ai.ag_ui.enabled {
            Some(AgUiProcessor::new(
                config.ai.ag_ui.validate_events,
                config.ai.ag_ui.allowed_events.clone(),
            ))
        } else {
            None
        };

        let a2ui_processor = if config.ai.a2ui.enabled {
            Some(A2UiProcessor::new(config.ai.a2ui.allowed_intents.clone()))
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
            user_scopes,
            metadata,
            theme,
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
                    // Accumulate into line_buffer + process the complete line
                    self.line_buffer.extend_from_slice(&chunk[pos..pos + newline_pos]);
                    let line = std::mem::take(&mut self.line_buffer);
                    let line_str = String::from_utf8_lossy(&line);

                    if let Some(processed) = self.process_line(&line_str) {
                        output.extend_from_slice(processed.as_bytes());
                        output.push(b'\n');
                    }

                    pos += newline_pos + 1;
                }
                None => {
                    // No newline in this chunk — buffer it
                    self.line_buffer.extend_from_slice(&chunk[pos..]);
                    break;
                }
            }
        }

        self.metrics.duration_ms = self.started_at.elapsed().as_millis() as u64;
        self.metrics.estimated_tokens = self.token_counter.estimated_tokens();

        Some(Bytes::from(output))
    }

    /// Process a single SSE line. Returns the line to forward, or `None` to drop.
    fn process_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim_end_matches('\r');

        // Empty line = end of SSE event
        if line.is_empty() {
            return self.flush_event().map(|s| format!("{s}\n"));
        }

        // SSE field: `data: ...`
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            self.current_event_data.push(data.to_string());
            return Some(line.to_string());
        }

        // SSE field: `event: ...`
        if let Some(event_type) = line.strip_prefix("event:") {
            self.current_event_type = Some(event_type.trim().to_string());
            return Some(line.to_string());
        }

        // SSE comments `:`
        if line.starts_with(':') {
            return Some(line.to_string());
        }

        // Other fields (id:, retry:) — pass through
        Some(line.to_string())
    }

    /// Called when we hit a blank line (end of SSE event).
    ///
    /// If no AG-UI/A2UI processing, returns an empty string (event separator).
    /// Otherwise tries to parse and filter the data.
    fn flush_event(&mut self) -> Option<String> {
        let data_lines = std::mem::take(&mut self.current_event_data);
        let _event_type = self.current_event_type.take();

        if data_lines.is_empty() {
            return Some(String::new()); // empty SSE separator
        }

        let data_str = data_lines.join("\n");

        // [DONE] sentinel — always pass through
        if data_str.trim() == "[DONE]" {
            self.metrics.total_events += 1;
            self.metrics.passed_events += 1;
            return Some(format!("data: {data_str}"));
        }

        self.metrics.total_events += 1;

        // Try AG-UI processing
        if let Some(ag_ui_proc) = &self.ag_ui_processor {
            if let Some(event) = AgUiEvent::from_json(&data_str) {
                self.token_counter.count_event(&event);

                let meta = self.metadata.clone();
                match ag_ui_proc.process(event, meta) {
                    None => {
                        self.metrics.dropped_events += 1;
                        return None; // drop this event
                    }
                    Some(processed) => {
                        self.metrics.passed_events += 1;
                        let json = processed.to_json();
                        return Some(format!("data: {json}"));
                    }
                }
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
                        return Some(format!("data: {json}"));
                    }
                }
            }
        }

        // No AI processing or couldn't parse — pass through as-is
        self.metrics.passed_events += 1;
        Some(format!("data: {data_str}"))
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
    use crate::config::types::{AiStreamConfig, AgUiConfig, StreamConfig};

    fn passthrough_config() -> StreamConfig {
        StreamConfig {
            enabled: true,
            protocol: "sse".to_string(),
            ai: AiStreamConfig::default(),
        }
    }

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
}
