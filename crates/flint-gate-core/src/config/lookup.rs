/// Async lookup registry for `{{ lookup:name(arg) }}` template expressions.
///
/// Lookups are async functions registered by name. The pipeline pre-resolves
/// all lookup expressions found in hook templates before rendering, so the
/// sync `TemplateEngine::render` can read results from `TemplateContext.lookups`.
///
/// Built-in lookups:
/// - `usage_budget(user_id)` — lifetime token total for a user from usage_events
use crate::config::template::TemplateContext;
use crate::db::Database;
use regex::Regex;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use tracing::debug;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type LookupFn = Arc<dyn Fn(String) -> BoxFuture<String> + Send + Sync>;

static LOOKUP_PATTERN: OnceLock<Regex> = OnceLock::new();

fn lookup_pattern() -> &'static Regex {
    LOOKUP_PATTERN.get_or_init(|| {
        Regex::new(r"\{\{\s*lookup:(\w+)\(([^)]*)\)\s*\}\}").expect("valid lookup regex")
    })
}

/// Registry of named async lookup functions.
pub struct LookupRegistry {
    handlers: HashMap<String, LookupFn>,
}

impl LookupRegistry {
    /// Create a new registry, pre-registering built-in lookups that need DB access.
    pub fn new(db: Option<Arc<Database>>) -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
        };
        if let Some(db) = db {
            registry.register_builtin_usage_budget(db);
        }
        registry
    }

    /// Register a named lookup handler.
    #[allow(dead_code)]
    pub fn register(&mut self, name: impl Into<String>, handler: LookupFn) {
        self.handlers.insert(name.into(), handler);
    }

    /// Scan `templates` for `{{ lookup:name(arg_expr) }}` patterns. For each
    /// found expression:
    /// 1. Resolve `arg_expr` against `ctx` using `TemplateEngine`
    /// 2. Call the registered handler with the resolved argument
    /// 3. Store `"name(resolved_arg)"` → result in the returned map
    ///
    /// The returned map is meant to be set as `ctx.lookups` before rendering.
    pub async fn resolve_all(
        &self,
        templates: &[&str],
        ctx: &TemplateContext,
    ) -> HashMap<String, String> {
        let re = lookup_pattern();
        let mut results = HashMap::new();

        for template in templates {
            for caps in re.captures_iter(template) {
                let name = &caps[1];
                let arg_expr = caps[2].trim();

                // Resolve the argument expression (may itself be a template expression)
                let resolved_arg = if arg_expr.contains('.') || arg_expr == "request_id" {
                    // Looks like a context path — render it
                    crate::config::template::TemplateEngine::render(
                        &format!("{{{{ {arg_expr} }}}}"),
                        ctx,
                    )
                } else {
                    arg_expr.to_string()
                };

                let cache_key = format!("{name}({resolved_arg})");
                if results.contains_key(&cache_key) {
                    continue; // already resolved this combination
                }

                let value = self.resolve_one(name, &resolved_arg).await;
                debug!(name, arg = %resolved_arg, value = %value, "lookup resolved");
                results.insert(cache_key, value);
            }
        }

        results
    }

    async fn resolve_one(&self, name: &str, arg: &str) -> String {
        match self.handlers.get(name) {
            Some(handler) => handler(arg.to_string()).await,
            None => {
                debug!(name, "no lookup handler registered");
                String::new()
            }
        }
    }

    /// Register the built-in `usage_budget(user_id)` lookup.
    fn register_builtin_usage_budget(&mut self, db: Arc<Database>) {
        let handler: LookupFn = Arc::new(move |user_id: String| {
            let db = db.clone();
            Box::pin(async move {
                match db.get_user_token_total(&user_id).await {
                    Ok(total) => total.to_string(),
                    Err(e) => {
                        tracing::warn!(error = %e, user_id, "usage_budget lookup failed");
                        String::new()
                    }
                }
            })
        });
        self.handlers.insert("usage_budget".to_string(), handler);
    }
}

/// Collect all template strings from a route's pre-request hook configs.
/// Used by the pipeline to feed into `LookupRegistry::resolve_all`.
///
/// For `MaxTokenBudget` hooks, synthesizes a `{{ lookup:usage_budget(expr) }}`
/// template so the registry pre-resolves the user's token total before hooks run.
pub fn collect_hook_templates(hooks: &[crate::config::types::PreRequestHook]) -> Vec<String> {
    let mut out = Vec::new();
    for hook in hooks {
        match hook {
            crate::config::types::PreRequestHook::ClaimsEnhancement { config } => {
                for v in config.inject_headers.values() {
                    out.push(v.clone());
                }
            }
            crate::config::types::PreRequestHook::BodyTransform { config } => {
                for v in config.set_fields.values() {
                    out.push(v.clone());
                }
            }
            crate::config::types::PreRequestHook::MaxTokenBudget { config } => {
                // Only lifetime budgets use the pre-resolved DB lookup. Windowed
                // budgets are resolved inline in the pipeline (Redis / windowed
                // Postgres), so synthesizing a lifetime lookup for them would be
                // a wasted all-time SUM query.
                if config.window == crate::config::types::BudgetWindow::Lifetime {
                    out.push(format!(
                        "{{{{ lookup:usage_budget({}) }}}}",
                        config.user_id_expr
                    ));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::template::TemplateContext;
    use serde_json::json;
    use std::collections::HashMap;

    fn ctx() -> TemplateContext {
        let identity = json!({"id": "user-42", "traits": {}});
        TemplateContext::new(identity, json!({}), "req-1".to_string(), HashMap::new())
    }

    #[tokio::test]
    async fn unknown_lookup_returns_empty() {
        let registry = LookupRegistry::new(None);
        let results = registry
            .resolve_all(&["{{ lookup:unknown(identity.id) }}"], &ctx())
            .await;
        assert_eq!(
            results.get("unknown(user-42)").map(String::as_str),
            Some("")
        );
    }

    #[tokio::test]
    async fn custom_handler_resolves() {
        let mut registry = LookupRegistry::new(None);
        registry.register(
            "greet",
            Arc::new(|arg: String| Box::pin(async move { format!("hello {arg}") })),
        );
        let results = registry
            .resolve_all(&["{{ lookup:greet(identity.id) }}"], &ctx())
            .await;
        assert_eq!(
            results.get("greet(user-42)").map(String::as_str),
            Some("hello user-42")
        );
    }

    #[tokio::test]
    async fn no_lookups_in_template_returns_empty_map() {
        let registry = LookupRegistry::new(None);
        let results = registry
            .resolve_all(&["{{ identity.id }}", "plain text"], &ctx())
            .await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn deduplicates_same_expression() {
        let mut registry = LookupRegistry::new(None);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_clone = calls.clone();
        registry.register(
            "counter",
            Arc::new(move |arg: String| {
                let c = calls_clone.clone();
                Box::pin(async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    arg
                })
            }),
        );
        let templates = [
            "{{ lookup:counter(identity.id) }}",
            "{{ lookup:counter(identity.id) }}", // duplicate
        ];
        registry.resolve_all(&templates, &ctx()).await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
