/// A2UI (Agent-to-UI) intent-driven protocol types and processing.
///
/// A2UI events are SSE frames with an `intent` field that commands the
/// frontend to perform actions (render components, navigate, show modals, etc.)
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Known A2UI intent type strings.
pub const INTENT_RENDER_COMPONENT: &str = "render_component";
pub const INTENT_UPDATE_STATE: &str = "update_state";
pub const INTENT_NAVIGATE: &str = "navigate";
pub const INTENT_SHOW_MODAL: &str = "show_modal";
pub const INTENT_SHOW_TOAST: &str = "show_toast";
pub const INTENT_REQUEST_INPUT: &str = "request_input";
pub const INTENT_STREAM_CONTENT: &str = "stream_content";

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

/// Processes A2UI events: filters by intent and scope.
#[derive(Clone)]
pub struct A2UiProcessor {
    allowed_intents: Option<HashSet<String>>,
}

impl A2UiProcessor {
    pub fn new(allowed_intents: Vec<String>) -> Self {
        let allowed = if !allowed_intents.is_empty() {
            Some(allowed_intents.into_iter().collect())
        } else {
            None
        };
        Self { allowed_intents: allowed }
    }

    /// Process an A2UI event, applying intent filtering and optional scope check.
    ///
    /// Returns `None` if the event should be dropped.
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

        // Inject theme for render_component
        if let Some(theme_value) = theme {
            event.inject_theme(theme_value);
        }

        Some(event)
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
}
