/// Route matching engine with glob-to-regex compilation and site scoping.
///
/// Routes are compiled once and reused across requests. Hot-reload swaps the
/// entire `Router` under an `Arc<RwLock<Router>>`.
use crate::config::types::{GateConfig, RouteConfig, SiteConfig};
use crate::db::DbRoute;
use regex::Regex;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Thread-safe shared router — swapped atomically on config reload.
pub type SharedRouter = Arc<RwLock<Router>>;

/// A route with pre-compiled regex for its path pattern.
#[derive(Debug, Clone)]
pub struct CompiledRoute {
    /// The original route configuration.
    pub config: RouteConfig,
    /// Pre-compiled regex for the path pattern.
    pub path_regex: Regex,
    /// Site this route belongs to.
    pub site: SiteConfig,
}

/// The main route matching engine.
#[derive(Debug)]
pub struct Router {
    routes: Vec<CompiledRoute>,
}

impl Router {
    /// Build a `Router` from a [`GateConfig`], compiling all route patterns from
    /// the config's own YAML routes.
    pub fn from_config(config: &GateConfig) -> Self {
        Self::from_config_with_routes(config, config.routes.clone())
    }

    /// Build a `Router` from a [`GateConfig`] (for sites/scoping) and an explicit
    /// route set — e.g. the merged (YAML + DB) routes from [`merge_routes`]. This
    /// is the shared compile core; `from_config` and `from_config_and_db_routes`
    /// both funnel through it, so callers that have already merged (and linted)
    /// the route set don't recompute the merge.
    pub fn from_config_with_routes(config: &GateConfig, route_set: Vec<RouteConfig>) -> Self {
        let site_map: std::collections::HashMap<&str, &SiteConfig> =
            config.sites.iter().map(|s| (s.id.as_str(), s)).collect();

        let mut routes: Vec<CompiledRoute> = route_set
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|route| {
                let site = match site_map.get(route.site.as_str()) {
                    Some(s) => (*s).clone(),
                    None => {
                        tracing::warn!(
                            route_id = %route.id,
                            site_id = %route.site,
                            "route references unknown site — skipping"
                        );
                        return None;
                    }
                };

                let pattern = glob_to_regex(&route.route_match.path);
                match Regex::new(&pattern) {
                    Ok(re) => Some(CompiledRoute {
                        config: route.clone(),
                        path_regex: re,
                        site,
                    }),
                    Err(e) => {
                        tracing::error!(
                            route_id = %route.id,
                            pattern = %pattern,
                            error = %e,
                            "failed to compile route regex — skipping"
                        );
                        None
                    }
                }
            })
            .collect();

        // Sort by priority (descending) then by path specificity (longer pattern first)
        routes.sort_by(|a, b| {
            b.config.priority.cmp(&a.config.priority).then(
                b.config
                    .route_match
                    .path
                    .len()
                    .cmp(&a.config.route_match.path.len()),
            )
        });

        Self { routes }
    }

    /// Build a `Router` from a [`GateConfig`] merged with DB-sourced routes.
    ///
    /// DB routes are deserialized from their stored `config` JSON blob and
    /// merged into the YAML routes. A DB route with the same `id` as a YAML
    /// route replaces it; otherwise it is appended. When `db_routes` is empty
    /// this is equivalent to `from_config`.
    pub fn from_config_and_db_routes(config: &GateConfig, db_routes: &[DbRoute]) -> Self {
        Self::from_config_with_routes(config, merge_routes(config, db_routes))
    }

    /// Find the best matching route for a request.
    ///
    /// Matches on `(host, path, method)`. Returns the first match after priority sort.
    pub fn match_route(&self, host: &str, path: &str, method: &str) -> Option<&CompiledRoute> {
        for route in &self.routes {
            // Site domain check
            if !route.site.domains.is_empty()
                && !route.site.domains.iter().any(|d| d == host || d == "*")
            {
                continue;
            }

            // Route-level host check
            if let Some(route_host) = &route.config.route_match.host {
                if !host_matches(route_host, host) {
                    continue;
                }
            }

            // Path pattern check
            if !route.path_regex.is_match(path) {
                continue;
            }

            // Method check (empty = any method)
            if !route.config.route_match.methods.is_empty() {
                let method_upper = method.to_uppercase();
                if !route
                    .config
                    .route_match
                    .methods
                    .iter()
                    .any(|m| m.to_uppercase() == method_upper)
                {
                    continue;
                }
            }

            debug!(
                route_id = %route.config.id,
                host = %host,
                path = %path,
                method = %method,
                "route matched"
            );
            return Some(route);
        }
        None
    }

    /// Resolve the upstream URL for a matched route and request path.
    ///
    /// - Route-level `upstream` → used as the full target URL
    /// - Site-level `default_upstream` → used as base, request path appended
    pub fn resolve_upstream(route: &CompiledRoute, request_path_and_query: &str) -> Option<String> {
        if let Some(upstream) = &route.config.upstream {
            return Some(upstream.clone());
        }
        if let Some(base) = &route.site.default_upstream {
            let base = base.trim_end_matches('/');
            let path = if request_path_and_query.starts_with('/') {
                request_path_and_query.to_string()
            } else {
                format!("/{request_path_and_query}")
            };
            return Some(format!("{base}{path}"));
        }
        None
    }

    /// Total number of compiled routes.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Iterate over the IDs of all compiled routes.
    pub fn route_ids(&self) -> impl Iterator<Item = String> + '_ {
        self.routes.iter().map(|r| r.config.id.clone())
    }
}

/// Merge YAML routes with DB-sourced routes into the effective route set the
/// router serves. Pure — the same set that [`Router::from_config_and_db_routes`]
/// compiles, surfaced so callers (e.g. the agent-governance lint) can inspect the
/// *served* routes, not just YAML. A disabled DB row removes the same-id YAML
/// route; an enabled DB row overrides the same-id YAML route, else is appended
/// (DB wins on id collision). Undeserializable DB rows are WARN-skipped, exactly
/// as during router build.
pub fn merge_routes(config: &GateConfig, db_routes: &[DbRoute]) -> Vec<RouteConfig> {
    // Start with YAML routes, keyed by id for O(1) override lookup.
    let mut yaml_by_id: std::collections::HashMap<String, RouteConfig> = config
        .routes
        .iter()
        .map(|r| (r.id.clone(), r.clone()))
        .collect();

    // Deserialize and merge DB routes. DB wins on id collision.
    let mut db_parsed: Vec<RouteConfig> = Vec::new();
    for db_route in db_routes {
        if !db_route.enabled {
            yaml_by_id.remove(&db_route.id);
            continue;
        }
        match serde_json::from_value::<RouteConfig>(db_route.config.clone()) {
            Ok(mut rc) => {
                rc.priority = db_route.priority;
                rc.enabled = db_route.enabled;
                yaml_by_id.remove(&rc.id); // DB overrides YAML
                db_parsed.push(rc);
            }
            Err(e) => {
                tracing::warn!(
                    route_id = %db_route.id,
                    error = %e,
                    "failed to deserialize DB route — skipping"
                );
            }
        }
    }

    yaml_by_id.into_values().chain(db_parsed).collect()
}

/// Check if a host header matches a route-level host pattern.
///
/// Supports exact match and `*.example.com` wildcard suffix.
/// Port is stripped from the host header before comparison.
/// Comparison is case-insensitive.
fn host_matches(pattern: &str, host_header: &str) -> bool {
    let host_no_port = host_header.split(':').next().unwrap_or(host_header);
    let pattern_lower = pattern.to_lowercase();
    let host_lower = host_no_port.to_lowercase();

    if let Some(suffix) = pattern_lower.strip_prefix('*') {
        // *.example.com → suffix is ".example.com"
        host_lower.ends_with(suffix)
    } else {
        pattern_lower == host_lower
    }
}

/// Convert a glob path pattern to a regex string.
///
/// Rules:
/// - `**` matches any path segment sequence (including `/`)
/// - `*` matches any path segment characters (not `/`)
/// - `?` matches a single character (not `/`)
/// - All other regex metacharacters are escaped
pub fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::from("^");
    let mut chars = glob.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second '*'
                                  // Match any sequence including '/'
                    regex.push_str(".*");
                } else {
                    // Match any sequence excluding '/'
                    regex.push_str("[^/]*");
                }
            }
            '?' => regex.push_str("[^/]"),
            // Escape regex metacharacters
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }

    regex.push('$');
    regex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_simple_path() {
        let re = Regex::new(&glob_to_regex("/health")).unwrap();
        assert!(re.is_match("/health"));
        assert!(!re.is_match("/healthz"));
        assert!(!re.is_match("/health/check"));
    }

    #[test]
    fn glob_double_star() {
        let re = Regex::new(&glob_to_regex("/api/**")).unwrap();
        assert!(re.is_match("/api/"));
        assert!(re.is_match("/api/v1/chat"));
        assert!(re.is_match("/api/v1/chat/completions"));
        assert!(!re.is_match("/other/path"));
    }

    #[test]
    fn glob_single_star() {
        let re = Regex::new(&glob_to_regex("/api/*/data")).unwrap();
        assert!(re.is_match("/api/v1/data"));
        assert!(!re.is_match("/api/v1/extra/data"));
    }

    #[test]
    fn glob_question_mark() {
        let re = Regex::new(&glob_to_regex("/v?")).unwrap();
        assert!(re.is_match("/v1"));
        assert!(re.is_match("/v2"));
        assert!(!re.is_match("/v12"));
    }

    #[test]
    fn router_matches_correct_route() {
        use crate::config::types::*;

        let config = GateConfig {
            sites: vec![SiteConfig {
                id: "app".to_string(),
                domains: vec!["app.example.com".to_string()],
                default_auth: None,
                default_upstream: Some("http://backend:3000".to_string()),
            }],
            routes: vec![RouteConfig {
                id: "chat".to_string(),
                site: "app".to_string(),
                route_match: RouteMatch {
                    path: "/api/chat/**".to_string(),
                    methods: vec!["POST".to_string()],
                    host: None,
                },
                upstream: Some("http://llm:8000/chat".to_string()),
                auth: None,
                hooks: HooksConfig::default(),
                stream: StreamConfig::default(),
                priority: 0,
                enabled: true,
            }],
            ..Default::default()
        };

        let router = Router::from_config(&config);
        assert_eq!(router.route_count(), 1);

        // Should match
        assert!(router
            .match_route("app.example.com", "/api/chat/completions", "POST")
            .is_some());

        // Wrong method
        assert!(router
            .match_route("app.example.com", "/api/chat/completions", "GET")
            .is_none());

        // Wrong host
        assert!(router
            .match_route("other.example.com", "/api/chat/completions", "POST")
            .is_none());
    }

    #[test]
    fn resolve_upstream_route_level() {
        use crate::config::types::*;

        let site = SiteConfig {
            id: "s".to_string(),
            domains: vec![],
            default_auth: None,
            default_upstream: Some("http://default:3000".to_string()),
        };
        let route = CompiledRoute {
            config: RouteConfig {
                id: "r".to_string(),
                site: "s".to_string(),
                route_match: RouteMatch::default(),
                upstream: Some("http://specific:8000/v1".to_string()),
                auth: None,
                hooks: HooksConfig::default(),
                stream: StreamConfig::default(),
                priority: 0,
                enabled: true,
            },
            path_regex: Regex::new("^.*$").unwrap(),
            site,
        };

        assert_eq!(
            Router::resolve_upstream(&route, "/any/path"),
            Some("http://specific:8000/v1".to_string())
        );
    }

    #[test]
    fn resolve_upstream_site_default() {
        use crate::config::types::*;

        let site = SiteConfig {
            id: "s".to_string(),
            domains: vec![],
            default_auth: None,
            default_upstream: Some("http://default:3000".to_string()),
        };
        let route = CompiledRoute {
            config: RouteConfig {
                id: "r".to_string(),
                site: "s".to_string(),
                route_match: RouteMatch::default(),
                upstream: None,
                auth: None,
                hooks: HooksConfig::default(),
                stream: StreamConfig::default(),
                priority: 0,
                enabled: true,
            },
            path_regex: Regex::new("^.*$").unwrap(),
            site,
        };

        assert_eq!(
            Router::resolve_upstream(&route, "/api/v1"),
            Some("http://default:3000/api/v1".to_string())
        );
    }

    #[test]
    fn db_route_overrides_yaml_route() {
        use crate::config::types::*;
        use crate::db::DbRoute;

        let site = SiteConfig {
            id: "app".to_string(),
            domains: vec!["app.example.com".to_string()],
            default_auth: None,
            default_upstream: Some("http://backend:3000".to_string()),
        };

        let yaml_route = RouteConfig {
            id: "chat".to_string(),
            site: "app".to_string(),
            route_match: RouteMatch {
                path: "/api/chat/**".to_string(),
                methods: vec!["POST".to_string()],
                host: None,
            },
            upstream: Some("http://yaml-llm:8000".to_string()),
            auth: None,
            hooks: HooksConfig::default(),
            stream: StreamConfig::default(),
            priority: 0,
            enabled: true,
        };

        // DB version of same route with different upstream
        let db_route = DbRoute {
            id: "chat".to_string(),
            config: serde_json::to_value(RouteConfig {
                upstream: Some("http://db-llm:9000".to_string()),
                ..yaml_route.clone()
            })
            .unwrap(),
            priority: 10,
            enabled: true,
        };

        let config = GateConfig {
            sites: vec![site],
            routes: vec![yaml_route],
            ..Default::default()
        };

        let router = Router::from_config_and_db_routes(&config, &[db_route]);
        assert_eq!(router.route_count(), 1);

        let matched = router
            .match_route("app.example.com", "/api/chat/completions", "POST")
            .unwrap();
        // DB route wins — upstream is the DB-sourced value
        assert_eq!(
            matched.config.upstream.as_deref(),
            Some("http://db-llm:9000")
        );
    }

    #[test]
    fn db_route_disabled_removes_yaml_route() {
        use crate::config::types::*;
        use crate::db::DbRoute;

        let site = SiteConfig {
            id: "s".to_string(),
            domains: vec!["example.com".to_string()],
            default_auth: None,
            default_upstream: None,
        };
        let yaml_route = RouteConfig {
            id: "r".to_string(),
            site: "s".to_string(),
            route_match: RouteMatch {
                path: "/api/**".to_string(),
                methods: vec!["GET".to_string()],
                host: None,
            },
            upstream: Some("http://upstream".to_string()),
            auth: None,
            hooks: HooksConfig::default(),
            stream: StreamConfig::default(),
            priority: 0,
            enabled: true,
        };
        let db_route = DbRoute {
            id: "r".to_string(),
            config: serde_json::json!({}),
            priority: 0,
            enabled: false, // disabled in DB
        };

        let config = GateConfig {
            sites: vec![site],
            routes: vec![yaml_route],
            ..Default::default()
        };

        let router = Router::from_config_and_db_routes(&config, &[db_route]);
        // Route was disabled in DB — not compiled
        assert_eq!(router.route_count(), 0);
    }

    #[test]
    fn host_matches_exact() {
        assert!(host_matches("api.example.com", "api.example.com"));
        assert!(!host_matches("api.example.com", "other.example.com"));
    }

    #[test]
    fn host_matches_wildcard() {
        assert!(host_matches("*.example.com", "api.example.com"));
        assert!(host_matches("*.example.com", "chat.api.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "other.org"));
    }

    #[test]
    fn host_matches_strips_port() {
        assert!(host_matches("api.example.com", "api.example.com:8080"));
        assert!(host_matches("*.example.com", "api.example.com:443"));
    }

    #[test]
    fn host_matches_case_insensitive() {
        assert!(host_matches("API.Example.COM", "api.example.com"));
        assert!(host_matches("*.Example.com", "API.example.COM"));
    }

    #[test]
    fn route_level_host_filter() {
        use crate::config::types::*;

        let config = GateConfig {
            sites: vec![SiteConfig {
                id: "app".to_string(),
                domains: vec![], // empty = match any host at site level
                default_auth: None,
                default_upstream: Some("http://backend:3000".to_string()),
            }],
            routes: vec![
                RouteConfig {
                    id: "api-only".to_string(),
                    site: "app".to_string(),
                    route_match: RouteMatch {
                        path: "/api/**".to_string(),
                        methods: vec![],
                        host: Some("api.example.com".to_string()),
                    },
                    upstream: Some("http://api-backend:3001".to_string()),
                    auth: None,
                    hooks: HooksConfig::default(),
                    stream: StreamConfig::default(),
                    priority: 10,
                    enabled: true,
                },
                RouteConfig {
                    id: "wildcard-host".to_string(),
                    site: "app".to_string(),
                    route_match: RouteMatch {
                        path: "/api/**".to_string(),
                        methods: vec![],
                        host: Some("*.example.com".to_string()),
                    },
                    upstream: Some("http://wildcard-backend:3002".to_string()),
                    auth: None,
                    hooks: HooksConfig::default(),
                    stream: StreamConfig::default(),
                    priority: 5,
                    enabled: true,
                },
            ],
            ..Default::default()
        };

        let router = Router::from_config(&config);

        // Exact host match — highest priority route wins
        let matched = router
            .match_route("api.example.com", "/api/data", "GET")
            .unwrap();
        assert_eq!(matched.config.id, "api-only");

        // Wildcard host match — second route
        let matched = router
            .match_route("chat.example.com", "/api/data", "GET")
            .unwrap();
        assert_eq!(matched.config.id, "wildcard-host");

        // Non-matching host — no route
        assert!(router
            .match_route("other.org", "/api/data", "GET")
            .is_none());

        // Port stripping
        let matched = router
            .match_route("api.example.com:8443", "/api/data", "GET")
            .unwrap();
        assert_eq!(matched.config.id, "api-only");
    }

    // ── merged-set governance lint (DB-sourced routes) ───────────────────────

    /// A GateConfig with a JWKS-backed (jwt) provider + one site, and NO YAML
    /// routes — so any route reaching the lint comes from the DB merge.
    fn jwt_config_no_routes() -> crate::config::types::GateConfig {
        serde_yaml::from_str(
            "auth_providers:\n  p:\n    type: jwt\n    jwks_url: \"https://idp/jwks\"\n    issuer: \"https://idp\"\n\
             sites:\n  - id: s\n    default_auth: p\n",
        )
        .expect("valid fixture")
    }

    /// A DB route row carrying an agent-reachable (jwt-authed) route with NO
    /// authorize hook — i.e. under-governed (a `NoAuthorizeHook` finding).
    fn under_governed_db_route(id: &str) -> crate::db::DbRoute {
        let rc = serde_json::json!({
            "id": id,
            "site": "s",
            "match": { "path": "/x" },
            "auth": "p",
        });
        crate::db::DbRoute {
            id: id.to_string(),
            config: rc,
            priority: 0,
            enabled: true,
        }
    }

    #[test]
    fn merge_routes_surfaces_db_only_routes_to_the_lint() {
        // The core of the change: a DB-only route appears in the merged set the
        // lint inspects, even though YAML has none.
        let config = jwt_config_no_routes();
        let merged = merge_routes(&config, &[under_governed_db_route("db-tool")]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "db-tool");
    }

    #[test]
    fn lint_flags_db_only_under_governed_agent_route() {
        // The YAML-only lint would MISS this (no YAML routes); the merged-set lint
        // catches it — the whole point of the change.
        let config = jwt_config_no_routes();
        assert!(
            config.agent_governance_lint().is_empty(),
            "YAML-only lint sees nothing (no YAML routes)"
        );
        let merged = merge_routes(&config, &[under_governed_db_route("db-tool")]);
        let findings = config.agent_governance_lint_routes(&merged);
        assert!(
            !findings.is_empty(),
            "merged-set lint must flag the DB-only agent-reachable route with no authorize hook"
        );
        assert!(findings.iter().any(|f| f.route_id == "db-tool"));
    }

    #[test]
    fn disabled_db_route_is_not_linted() {
        // A disabled DB row is removed from the merged set → nothing to flag.
        let config = jwt_config_no_routes();
        let mut db = under_governed_db_route("db-tool");
        db.enabled = false;
        let merged = merge_routes(&config, &[db]);
        assert!(merged.is_empty());
        assert!(config.agent_governance_lint_routes(&merged).is_empty());
    }
}
