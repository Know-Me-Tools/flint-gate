/// NDJSON (newline-delimited JSON) stream processor.
///
/// Splits the upstream response on `\n`, parses each line as a JSON object,
/// runs it through the AG-UI/A2UI event chain, and re-emits filtered events.
use crate::config::types::StreamConfig;
use crate::stream::a2ui::{A2UiEvent, A2UiProcessor};
use crate::stream::ag_ui::{AgUiEvent, AgUiProcessor, AgUiTokenCounter};
use crate::stream::StreamMetrics;
use bytes::Bytes;
use std::time::Instant;

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
}

impl NdjsonStreamProcessor {
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

    /// Process a complete NDJSON line.
    fn process_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        self.metrics.total_events += 1;

        // Try AG-UI processing
        if let Some(ref ag_ui_proc) = self.ag_ui_processor {
            if let Some(event) = AgUiEvent::from_json(line) {
                self.token_counter.count_event(&event);
                let meta = self.metadata.clone();
                match ag_ui_proc.process(event, meta) {
                    None => {
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

        // Try A2UI processing
        if let Some(ref a2ui_proc) = self.a2ui_processor {
            if let Some(event) = A2UiEvent::from_json(line) {
                match a2ui_proc.process(event, &self.user_scopes, self.theme.clone()) {
                    None => {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{AiStreamConfig, StreamConfig};
    use crate::stream::StreamProcessor;

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
}
