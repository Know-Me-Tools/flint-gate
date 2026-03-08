/// Template engine for `{{ expression }}` substitution in hook configurations.
///
/// Supports:
/// - `{{ identity.id }}` — dot-path into Identity fields
/// - `{{ body.field }}` — dot-path into the JSON request body
/// - `{{ request_id }}` — the per-request UUID
/// - `{{ api_key.client_id }}` — API key metadata
/// - `{{ coalesce(a, b, 'fallback') }}` — first non-empty value
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

static TEMPLATE_REGEX: OnceLock<Regex> = OnceLock::new();

fn template_regex() -> &'static Regex {
    TEMPLATE_REGEX.get_or_init(|| {
        Regex::new(r"\{\{\s*(.+?)\s*\}\}").expect("valid template regex")
    })
}

/// Per-request context available to template expressions.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    /// Serialized identity fields (id, traits, metadata_public, etc.)
    pub identity: Value,
    /// Parsed JSON request body.
    pub body: Value,
    /// Auto-generated UUID for the current request.
    pub request_id: String,
    /// API key metadata (client_id, scopes).
    pub api_key: HashMap<String, String>,
}

impl TemplateContext {
    /// Create a new template context.
    pub fn new(
        identity: Value,
        body: Value,
        request_id: String,
        api_key: HashMap<String, String>,
    ) -> Self {
        Self {
            identity,
            body,
            request_id,
            api_key,
        }
    }
}

/// Renders a template string against a [`TemplateContext`].
pub struct TemplateEngine;

impl TemplateEngine {
    /// Render all `{{ expression }}` placeholders in `template`.
    ///
    /// Unknown expressions are replaced with an empty string.
    pub fn render(template: &str, ctx: &TemplateContext) -> String {
        let re = template_regex();
        re.replace_all(template, |caps: &regex::Captures| {
            let expr = caps[1].trim();
            Self::eval_expr(expr, ctx)
        })
        .into_owned()
    }

    /// Evaluate a single expression against the context.
    fn eval_expr(expr: &str, ctx: &TemplateContext) -> String {
        // coalesce(a, b, 'literal')
        if expr.starts_with("coalesce(") && expr.ends_with(')') {
            let inner = &expr["coalesce(".len()..expr.len() - 1];
            return Self::eval_coalesce(inner, ctx);
        }

        // lookup:function_name(arg)
        if let Some(rest) = expr.strip_prefix("lookup:") {
            return Self::eval_lookup(rest, ctx);
        }

        // identity.*
        if let Some(path) = expr.strip_prefix("identity.") {
            return Self::resolve_dot_path(&ctx.identity, path);
        }

        // body.*
        if let Some(path) = expr.strip_prefix("body.") {
            return Self::resolve_dot_path(&ctx.body, path);
        }

        // api_key.*
        if let Some(key) = expr.strip_prefix("api_key.") {
            return ctx.api_key.get(key).cloned().unwrap_or_default();
        }

        // request_id
        if expr == "request_id" {
            return ctx.request_id.clone();
        }

        String::new()
    }

    /// Evaluate `coalesce(a, b, 'literal')` — return first non-empty resolved value.
    fn eval_coalesce(inner: &str, ctx: &TemplateContext) -> String {
        for part in Self::split_coalesce_args(inner) {
            let part = part.trim();
            let value = if (part.starts_with('\'') && part.ends_with('\''))
                || (part.starts_with('"') && part.ends_with('"'))
            {
                // String literal
                part[1..part.len() - 1].to_string()
            } else {
                Self::eval_expr(part, ctx)
            };
            if !value.is_empty() {
                return value;
            }
        }
        String::new()
    }

    /// Split `coalesce` arguments on commas, respecting quoted strings.
    fn split_coalesce_args(s: &str) -> Vec<&str> {
        let mut args = Vec::new();
        let mut depth = 0i32;
        let mut in_quote = false;
        let mut quote_char = ' ';
        let mut start = 0;

        for (i, c) in s.char_indices() {
            match c {
                '\'' | '"' if !in_quote => {
                    in_quote = true;
                    quote_char = c;
                }
                c if in_quote && c == quote_char => {
                    in_quote = false;
                }
                '(' if !in_quote => depth += 1,
                ')' if !in_quote => depth -= 1,
                ',' if !in_quote && depth == 0 => {
                    args.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        args.push(&s[start..]);
        args
    }

    /// Evaluate `lookup:name(arg)` — placeholder for the lookup registry.
    ///
    /// Returns empty string until a registry is wired in.
    fn eval_lookup(expr: &str, _ctx: &TemplateContext) -> String {
        tracing::debug!(expr, "lookup expression not resolved (no registry)");
        String::new()
    }

    /// Resolve a dot-separated path into a [`serde_json::Value`].
    ///
    /// `resolve_dot_path(json, "traits.email")` walks `json["traits"]["email"]`.
    pub fn resolve_dot_path(value: &Value, path: &str) -> String {
        let mut current = value;
        for segment in path.split('.') {
            match current {
                Value::Object(map) => match map.get(segment) {
                    Some(v) => current = v,
                    None => return String::new(),
                },
                Value::Array(arr) => {
                    if let Ok(idx) = segment.parse::<usize>() {
                        match arr.get(idx) {
                            Some(v) => current = v,
                            None => return String::new(),
                        }
                    } else {
                        return String::new();
                    }
                }
                _ => return String::new(),
            }
        }
        match current {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> TemplateContext {
        let identity = json!({
            "id": "user-123",
            "traits": {
                "email": "alice@example.com",
                "name": "Alice"
            },
            "metadata_public": {
                "org_id": "org-456"
            }
        });
        let body = json!({
            "model": "claude-sonnet-4-6",
            "messages": []
        });
        let mut api_key = HashMap::new();
        api_key.insert("client_id".to_string(), "client-abc".to_string());
        api_key.insert("scopes".to_string(), "chat,admin".to_string());

        TemplateContext::new(identity, body, "req-789".to_string(), api_key)
    }

    #[test]
    fn render_identity_id() {
        let result = TemplateEngine::render("{{ identity.id }}", &ctx());
        assert_eq!(result, "user-123");
    }

    #[test]
    fn render_identity_trait_nested() {
        let result = TemplateEngine::render("{{ identity.traits.email }}", &ctx());
        assert_eq!(result, "alice@example.com");
    }

    #[test]
    fn render_identity_metadata() {
        let result = TemplateEngine::render("{{ identity.metadata_public.org_id }}", &ctx());
        assert_eq!(result, "org-456");
    }

    #[test]
    fn render_body_field() {
        let result = TemplateEngine::render("{{ body.model }}", &ctx());
        assert_eq!(result, "claude-sonnet-4-6");
    }

    #[test]
    fn render_request_id() {
        let result = TemplateEngine::render("{{ request_id }}", &ctx());
        assert_eq!(result, "req-789");
    }

    #[test]
    fn render_api_key() {
        let result = TemplateEngine::render("{{ api_key.client_id }}", &ctx());
        assert_eq!(result, "client-abc");
    }

    #[test]
    fn render_coalesce_first_wins() {
        let result = TemplateEngine::render(
            "{{ coalesce(body.model, 'fallback') }}",
            &ctx(),
        );
        assert_eq!(result, "claude-sonnet-4-6");
    }

    #[test]
    fn render_coalesce_fallback() {
        let result = TemplateEngine::render(
            "{{ coalesce(body.missing_field, 'fallback-model') }}",
            &ctx(),
        );
        assert_eq!(result, "fallback-model");
    }

    #[test]
    fn render_unknown_expression() {
        let result = TemplateEngine::render("{{ unknown.thing }}", &ctx());
        assert_eq!(result, "");
    }

    #[test]
    fn render_multiple_expressions() {
        let result = TemplateEngine::render(
            "User={{ identity.id }} Org={{ identity.metadata_public.org_id }}",
            &ctx(),
        );
        assert_eq!(result, "User=user-123 Org=org-456");
    }

    #[test]
    fn render_no_expressions() {
        let result = TemplateEngine::render("plain string", &ctx());
        assert_eq!(result, "plain string");
    }

    #[test]
    fn resolve_dot_path_nested() {
        let v = serde_json::json!({"a": {"b": {"c": "deep"}}});
        assert_eq!(TemplateEngine::resolve_dot_path(&v, "a.b.c"), "deep");
    }
}
