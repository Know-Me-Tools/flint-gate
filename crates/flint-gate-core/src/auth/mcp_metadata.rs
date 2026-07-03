//! RFC 9728 Protected Resource Metadata + OAuth 2.1 `WWW-Authenticate`
//! discovery/step-up headers for the MCP Resource Server surface.
//!
//! - [`protected_resource_metadata`] builds the near-static JSON served at
//!   `/.well-known/oauth-protected-resource` (RFC 9728). It advertises this
//!   RS's canonical resource URI, the trusted Authorization Server issuers, and
//!   the supported scopes so an MCP client can bootstrap discovery.
//! - [`www_authenticate_discovery`] / [`www_authenticate_insufficient_scope`]
//!   build the `WWW-Authenticate: Bearer …` challenge headers an MCP client
//!   follows on `401` (find the metadata) and `403` (which scope to request).

use crate::config::types::{AuthProviderConfig, McpAuthConfig};
use crate::config::SharedConfig;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

/// Well-known path (relative) for the RFC 9728 metadata document.
pub const PROTECTED_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// Build the RFC 9728 Protected Resource Metadata JSON for an MCP provider.
///
/// Shape (RFC 9728 §2):
/// ```json
/// {
///   "resource": "https://rs.example/mcp",
///   "authorization_servers": ["https://as.example"],
///   "scopes_supported": ["read", "write"],
///   "bearer_methods_supported": ["header"]
/// }
/// ```
pub fn protected_resource_metadata(cfg: &McpAuthConfig) -> Value {
    json!({
        "resource": cfg.resource,
        "authorization_servers": cfg.authorization_servers,
        "scopes_supported": cfg.required_scopes,
        "bearer_methods_supported": ["header"],
    })
}

/// Select the MCP provider config whose metadata this endpoint serves.
///
/// Deployments typically configure a single MCP Resource Server; when several
/// are present we deterministically pick the lexicographically-smallest
/// provider name so the served document is stable across restarts rather than
/// dependent on `HashMap` iteration order.
fn primary_mcp_config(config: &crate::config::types::GateConfig) -> Option<&McpAuthConfig> {
    config
        .auth_providers
        .iter()
        .filter_map(|(name, cfg)| match cfg {
            AuthProviderConfig::Mcp(m) => Some((name, m)),
            _ => None,
        })
        .min_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, cfg)| cfg)
}

/// Axum handler for `GET /.well-known/oauth-protected-resource`.
///
/// Reads the live config so a hot-reload of `auth_providers` is reflected
/// without a restart. Returns `404` when no MCP provider is configured.
pub async fn protected_resource_metadata_handler(config: SharedConfig) -> Response {
    let guard = config.read().await;
    match primary_mcp_config(&guard) {
        Some(cfg) => Json(protected_resource_metadata(cfg)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no MCP resource server configured"})),
        )
            .into_response(),
    }
}

/// `WWW-Authenticate` value for a `401` on an MCP-protected route.
///
/// Points the client at the Protected Resource Metadata document so it can
/// discover the Authorization Server(s) and begin an OAuth flow. `metadata_url`
/// should be the absolute URL of [`PROTECTED_RESOURCE_METADATA_PATH`] on this
/// proxy (scheme+host derived from the inbound request).
pub fn www_authenticate_discovery(metadata_url: &str) -> String {
    format!("Bearer resource_metadata=\"{metadata_url}\"")
}

/// `WWW-Authenticate` value for a `403` insufficient-scope step-up.
///
/// Tells the client which scope(s) to request from the AS. `required` is
/// space-joined per the OAuth `scope` grammar.
pub fn www_authenticate_insufficient_scope(required: &[String]) -> String {
    let scope = required.join(" ");
    format!("Bearer error=\"insufficient_scope\", scope=\"{scope}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> McpAuthConfig {
        McpAuthConfig {
            jwks_url: "https://as.example/jwks".to_string(),
            issuer: Some("https://as.example".to_string()),
            audience: Some("https://rs.example/mcp".to_string()),
            resource: "https://rs.example/mcp".to_string(),
            authorization_servers: vec!["https://as.example".to_string()],
            required_scopes: vec!["read".to_string(), "write".to_string()],
            leeway_seconds: 5,
        }
    }

    #[test]
    fn metadata_has_rfc9728_shape() {
        let m = protected_resource_metadata(&cfg());
        assert_eq!(m["resource"], "https://rs.example/mcp");
        assert_eq!(m["authorization_servers"][0], "https://as.example");
        assert_eq!(m["scopes_supported"][0], "read");
        assert_eq!(m["scopes_supported"][1], "write");
        assert_eq!(m["bearer_methods_supported"][0], "header");
        // bearer_methods_supported must be exactly ["header"].
        assert_eq!(m["bearer_methods_supported"], serde_json::json!(["header"]));
    }

    #[test]
    fn metadata_empty_scopes_serializes_as_empty_array() {
        let mut c = cfg();
        c.required_scopes = vec![];
        let m = protected_resource_metadata(&c);
        assert_eq!(m["scopes_supported"], serde_json::json!([]));
    }

    #[test]
    fn discovery_header_format() {
        let h =
            www_authenticate_discovery("https://gate.example/.well-known/oauth-protected-resource");
        assert_eq!(
            h,
            "Bearer resource_metadata=\"https://gate.example/.well-known/oauth-protected-resource\""
        );
    }

    #[test]
    fn insufficient_scope_header_format() {
        let h = www_authenticate_insufficient_scope(&["read".to_string(), "write".to_string()]);
        assert_eq!(
            h,
            "Bearer error=\"insufficient_scope\", scope=\"read write\""
        );
    }

    #[test]
    fn insufficient_scope_header_single_scope() {
        let h = www_authenticate_insufficient_scope(&["admin".to_string()]);
        assert_eq!(h, "Bearer error=\"insufficient_scope\", scope=\"admin\"");
    }
}
