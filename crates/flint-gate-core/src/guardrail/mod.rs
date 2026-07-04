/// Input/output guardrails for requests proxied through Flint Gate.
///
/// A guardrail inspects an inbound request (headers + body) and decides whether
/// to allow, block, or annotate it. The trait is intentionally minimal so that
/// future guards (model-based classifiers, allowlists, etc.) plug in without
/// changing the pipeline.
use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The request attributes a guardrail may inspect.
#[derive(Debug, Clone, Default)]
pub struct GuardrailInput {
    pub request_id: String,
    pub route_id: String,
    pub principal_id: String,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Value,
}

/// The decision a guardrail returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailOutcome {
    /// Request may proceed unchanged.
    Allow,
    /// Request must be rejected before reaching the upstream.
    Block { reason: String },
    /// Request may proceed, but downstream telemetry should be tagged.
    Annotate { labels: Vec<String> },
}

impl GuardrailOutcome {
    /// True if the outcome is `Allow`.
    pub fn is_allow(&self) -> bool {
        matches!(self, GuardrailOutcome::Allow)
    }

    /// True if the outcome is `Block`.
    pub fn is_block(&self) -> bool {
        matches!(self, GuardrailOutcome::Block { .. })
    }
}

/// Guardrail implementations implement this trait.
#[async_trait]
pub trait Guardrail: Send + Sync {
    async fn inspect(&self, input: &GuardrailInput) -> GuardrailOutcome;
}

/// Configuration discriminant for the guardrail hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardrailConfig {
    /// Block requests whose body matches any of the configured regex patterns.
    Regex { config: RegexGuardrailConfig },
}

/// Per-hook configuration (shared across guard implementations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailHookConfig {
    /// The concrete guard to run.
    pub guard: GuardrailConfig,
    /// When `false`, a `Block` decision is logged but the request is allowed.
    #[serde(default = "default_true")]
    pub enforce: bool,
    /// Custom message returned in the response body on a blocked request.
    pub error_message: Option<String>,
}

impl Default for GuardrailHookConfig {
    fn default() -> Self {
        Self {
            guard: GuardrailConfig::Regex {
                config: RegexGuardrailConfig::default(),
            },
            enforce: true,
            error_message: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Reference regex guardrail.
///
/// Compiles each pattern once and matches against a flattened text
/// representation of the request body. This is a trivial, bundled guard meant
/// to validate the interface — heavier classifiers can be added later without
/// touching the hook plumbing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegexGuardrailConfig {
    /// Regex patterns. Invalid patterns are ignored with a warning at build time.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Whether matching is case-sensitive (default: false).
    #[serde(default)]
    pub case_sensitive: bool,
    /// Optional override message on block.
    pub error_message: Option<String>,
}

pub struct RegexGuardrail {
    patterns: Vec<Regex>,
    error_message: Option<String>,
}

impl RegexGuardrail {
    /// Build a regex guard from config. Patterns that fail to compile are
    /// dropped and logged; the guard still runs with the valid subset.
    pub fn new(config: &RegexGuardrailConfig) -> Self {
        let mut patterns = Vec::with_capacity(config.patterns.len());
        for pat in &config.patterns {
            let full = if config.case_sensitive {
                pat.clone()
            } else {
                format!("(?i){}", pat)
            };
            match Regex::new(&full) {
                Ok(re) => patterns.push(re),
                Err(e) => {
                    tracing::warn!(pattern = %pat, error = %e, "skipping invalid guardrail regex");
                }
            }
        }
        Self {
            patterns,
            error_message: config.error_message.clone(),
        }
    }

    fn body_text(body: &Value) -> String {
        match body {
            Value::String(s) => s.clone(),
            _ => body.to_string(),
        }
    }
}

#[async_trait]
impl Guardrail for RegexGuardrail {
    async fn inspect(&self, input: &GuardrailInput) -> GuardrailOutcome {
        let text = Self::body_text(&input.body);
        for re in &self.patterns {
            if let Some(m) = re.find(&text) {
                let reason = self
                    .error_message
                    .clone()
                    .unwrap_or_else(|| format!("guardrail matched pattern: {}", re.as_str()));
                let matched = m.as_str().chars().take(80).collect::<String>();
                tracing::info!(
                    request_id = %input.request_id,
                    route_id = %input.route_id,
                    pattern = %re.as_str(),
                    matched = %matched,
                    "guardrail block"
                );
                return GuardrailOutcome::Block { reason };
            }
        }
        GuardrailOutcome::Allow
    }
}

/// Build a guardrail instance from its config.
pub fn build_guardrail(config: &GuardrailConfig) -> Box<dyn Guardrail> {
    match config {
        GuardrailConfig::Regex { config } => Box::new(RegexGuardrail::new(config)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_with_body(body: Value) -> GuardrailInput {
        GuardrailInput {
            request_id: "req-1".to_string(),
            route_id: "route-a".to_string(),
            principal_id: "user-1".to_string(),
            method: "POST".to_string(),
            path: "/v1/chat".to_string(),
            headers: HashMap::new(),
            body,
        }
    }

    #[tokio::test]
    async fn regex_guard_allows_when_no_patterns_match() {
        let cfg = RegexGuardrailConfig {
            patterns: vec!["secret-key".to_string()],
            case_sensitive: false,
            error_message: None,
        };
        let guard = RegexGuardrail::new(&cfg);
        let input = input_with_body(Value::String("hello world".to_string()));
        let outcome = guard.inspect(&input).await;
        assert_eq!(outcome, GuardrailOutcome::Allow);
    }

    #[tokio::test]
    async fn regex_guard_blocks_on_body_match() {
        let cfg = RegexGuardrailConfig {
            patterns: vec!["password\\s*[:=]".to_string()],
            case_sensitive: false,
            error_message: Some("sensitive content detected".to_string()),
        };
        let guard = RegexGuardrail::new(&cfg);
        let input = input_with_body(Value::String("my password: 123".to_string()));
        let outcome = guard.inspect(&input).await;
        assert!(
            matches!(outcome, GuardrailOutcome::Block { reason } if reason == "sensitive content detected")
        );
    }

    #[tokio::test]
    async fn regex_guard_is_case_insensitive_by_default() {
        let cfg = RegexGuardrailConfig {
            patterns: vec!["forbidden".to_string()],
            case_sensitive: false,
            error_message: None,
        };
        let guard = RegexGuardrail::new(&cfg);
        let input = input_with_body(Value::String("FORBIDDEN word".to_string()));
        let outcome = guard.inspect(&input).await;
        assert!(outcome.is_block());
    }

    #[tokio::test]
    async fn regex_guard_case_sensitive_when_configured() {
        let cfg = RegexGuardrailConfig {
            patterns: vec!["forbidden".to_string()],
            case_sensitive: true,
            error_message: None,
        };
        let guard = RegexGuardrail::new(&cfg);
        let input = input_with_body(Value::String("FORBIDDEN word".to_string()));
        let outcome = guard.inspect(&input).await;
        assert!(outcome.is_allow());
    }

    #[tokio::test]
    async fn regex_guard_ignores_invalid_patterns() {
        let cfg = RegexGuardrailConfig {
            patterns: vec!["[invalid".to_string(), "bad".to_string()],
            case_sensitive: false,
            error_message: None,
        };
        let guard = RegexGuardrail::new(&cfg);
        let input = input_with_body(Value::String("this is bad".to_string()));
        let outcome = guard.inspect(&input).await;
        assert!(outcome.is_block());
    }

    #[test]
    fn guardrail_outcome_helpers() {
        assert!(GuardrailOutcome::Allow.is_allow());
        assert!(!GuardrailOutcome::Allow.is_block());
        let block = GuardrailOutcome::Block {
            reason: "x".to_string(),
        };
        assert!(block.is_block());
        assert!(!block.is_allow());
    }

    #[test]
    fn deserialize_regex_guardrail_hook() {
        let yaml = r#"
guard:
  type: regex
  config:
    patterns:
      - "secret"
      - "password"
    case_sensitive: true
    error_message: "blocked by guardrail"
enforce: false
"#;
        let hook: GuardrailHookConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!hook.enforce);
        assert_eq!(hook.error_message, None);
        match hook.guard {
            GuardrailConfig::Regex { config } => {
                assert_eq!(config.patterns, vec!["secret", "password"]);
                assert!(config.case_sensitive);
                assert_eq!(
                    config.error_message.as_deref(),
                    Some("blocked by guardrail")
                );
            }
        }
    }
}
