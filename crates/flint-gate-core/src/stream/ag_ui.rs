/// AG-UI (CopilotKit Agent-User Interface) protocol types and processing.
///
/// AG-UI events are delivered as SSE frames. Each frame's `data:` field
/// contains a JSON object with a `type` field identifying the event.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

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

/// Processes AG-UI events: validates against the allowed list, injects metadata.
#[derive(Clone)]
pub struct AgUiProcessor {
    allowed_events: Option<HashSet<String>>,
    validate: bool,
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
        }
    }

    /// Process an event. Returns `None` if the event should be dropped.
    pub fn process(
        &self,
        mut event: AgUiEvent,
        metadata: serde_json::Map<String, Value>,
    ) -> Option<AgUiEvent> {
        // Validate against allowed list
        if self.validate {
            if let Some(allowed) = &self.allowed_events {
                if !allowed.contains(&event.event_type) {
                    tracing::debug!(event_type = %event.event_type, "AG-UI event blocked by allow-list");
                    return None;
                }
            }
        }

        // Inject metadata
        if !metadata.is_empty() {
            event.inject_metadata(metadata);
        }

        Some(event)
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
}
