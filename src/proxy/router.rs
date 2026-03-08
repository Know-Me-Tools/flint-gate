/// Route matching engine with glob-to-regex compilation and site scoping.
///
/// Routes are compiled once and reused across requests. Hot-reload swaps the
/// entire `Router` under an `Arc<RwLock<Router>>`.
use crate::config::types::{GateConfig, RouteConfig, SiteConfig};
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
    /// Build a `Router` from a [`GateConfig`], compiling all route patterns.
    pub fn from_config(config: &GateConfig) -> Self {
        let site_map: std::collections::HashMap<&str, &SiteConfig> =
            config.sites.iter().map(|s| (s.id.as_str(), s)).collect();

        let mut routes: Vec<CompiledRoute> = config
            .routes
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
            b.config
                .priority
                .cmp(&a.config.priority)
                .then(b.config.route_match.path.len().cmp(&a.config.route_match.path.len()))
        });

        Self { routes }
    }

    /// Find the best matching route for a request.
    ///
    /// Matches on `(host, path, method)`. Returns the first match after priority sort.
    pub fn match_route(
        &self,
        host: &str,
        path: &str,
        method: &str,
    ) -> Option<&CompiledRoute> {
        for route in &self.routes {
            // Site domain check
            if !route.site.domains.is_empty()
                && !route.site.domains.iter().any(|d| d == host || d == "*")
            {
                continue;
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
}
