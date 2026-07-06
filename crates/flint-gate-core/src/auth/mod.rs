/// Authentication module — pluggable authenticator trait and built-in implementations.
pub mod api_key;
pub mod client_credentials;
pub mod http_body;
pub mod identity;
pub mod introspect;
pub mod jwks;
pub mod jwt_mint;
pub mod jwt_verify;
pub mod kratos;
pub mod mcp;
pub mod mcp_metadata;
pub mod oauth;
pub mod pkce;
pub mod token_exchange;

pub use api_key::ApiKeyAuthenticator;
pub use identity::Identity;
pub use jwt_mint::{JwtMinter, SharedJwtMinter};
pub use jwt_verify::JwtVerifyAuthenticator;
pub use kratos::KratosAuthenticator;
pub use mcp::McpAuthenticator;

use crate::config::types::AuthProviderConfig;
use crate::db::Database;
use async_trait::async_trait;
use http::request::Parts;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

/// Error returned by an authenticator.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum AuthError {
    /// The credential is missing or invalid — return 401.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// The token verified but lacks a required scope — return 403 with an
    /// `insufficient_scope` `WWW-Authenticate` challenge (OAuth 2.1 step-up).
    #[error("insufficient scope: requires {}", required.join(" "))]
    InsufficientScope { required: Vec<String> },
    /// The upstream auth provider is unreachable or returned an error — return 502.
    #[error("auth provider error: {0}")]
    ProviderError(String),
    /// Auth is not configured — proceed as anonymous.
    #[error("no auth configured")]
    NotConfigured,
}

/// How the request was authenticated.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    KratosSession,
    BearerJwt,
    /// MCP OAuth 2.1 access token (RFC 8707 audience-bound) with granted scopes.
    McpBearer {
        scopes: Vec<String>,
    },
    ApiKey {
        client_id: String,
        scopes: Vec<String>,
    },
    Anonymous,
}

/// Successful authentication result.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub identity: Identity,
    pub method: AuthMethod,
}

/// Pluggable authenticator trait — one implementation per auth provider type.
#[async_trait]
pub trait Authenticator: Send + Sync {
    /// Authenticate the incoming request (represented as `Parts`).
    ///
    /// Returns the resolved [`AuthResult`] on success or an [`AuthError`] on failure.
    async fn authenticate(&self, parts: &Parts) -> Result<AuthResult, AuthError>;
}

/// Anonymous authenticator — always succeeds with a configurable subject.
pub struct AnonymousAuthenticator {
    subject: String,
}

impl AnonymousAuthenticator {
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
        }
    }
}

#[async_trait]
impl Authenticator for AnonymousAuthenticator {
    async fn authenticate(&self, _parts: &Parts) -> Result<AuthResult, AuthError> {
        Ok(AuthResult {
            identity: Identity::anonymous(&self.subject),
            method: AuthMethod::Anonymous,
        })
    }
}

/// Build a map of named authenticators from the config's `auth_providers` section.
///
/// `db` is required by the API key authenticator; if it is `None` and an
/// `api_key` provider is configured, that provider will always return 503.
pub fn build_authenticators(
    providers: &HashMap<String, AuthProviderConfig>,
    http_client: &reqwest::Client,
    db: Option<Arc<Database>>,
) -> HashMap<String, Arc<dyn Authenticator>> {
    let mut map: HashMap<String, Arc<dyn Authenticator>> = HashMap::new();
    for (name, config) in providers {
        map.insert(
            name.clone(),
            build_authenticator(name, config, http_client, db.clone()),
        );
    }
    map
}

/// Build a single [`Authenticator`] from one provider config. Shared by
/// [`build_authenticators`] (proxy auth providers) and the admin-auth wiring so
/// the fail-closed construction rules (misconfigured MCP → `FailingAuthenticator`,
/// api_key without db → `FailingAuthenticator`) live in exactly one place.
///
/// `name` is used only for diagnostic logging.
pub fn build_authenticator(
    name: &str,
    config: &AuthProviderConfig,
    http_client: &reqwest::Client,
    db: Option<Arc<Database>>,
) -> Arc<dyn Authenticator> {
    match config {
        AuthProviderConfig::Kratos(cfg) => {
            Arc::new(KratosAuthenticator::new(cfg.clone(), http_client.clone()))
        }
        AuthProviderConfig::Anonymous(cfg) => {
            Arc::new(AnonymousAuthenticator::new(cfg.default_subject.clone()))
        }
        AuthProviderConfig::Jwt(cfg) => {
            Arc::new(JwtVerifyAuthenticator::new(cfg.clone(), http_client.clone()))
        }
        AuthProviderConfig::Mcp(cfg) => {
            // Fail CLOSED on a misconfigured MCP RS. Without an `audience`
            // the RFC 8707 confused-deputy check is a no-op (any signed
            // token would pass — C1); without an `issuer` we cannot pin the
            // trusted AS (M3). Refuse to build a permissive authenticator;
            // mirror the api_key-without-db pattern with a FailingAuthenticator.
            let mut missing: Vec<&str> = Vec::new();
            if cfg.audience.is_none() {
                missing.push("audience");
            }
            if cfg.issuer.is_none() {
                missing.push("issuer");
            }
            if missing.is_empty() {
                Arc::new(McpAuthenticator::new(cfg.clone(), http_client.clone()))
            } else {
                let fields = missing.join(", ");
                tracing::error!(
                    provider = %name,
                    missing = %fields,
                    "mcp provider missing required security fields ({fields}) — provider will always fail (fail-closed)"
                );
                Arc::new(FailingAuthenticator::new(format!(
                    "mcp provider requires {fields} to be configured"
                )))
            }
        }
        AuthProviderConfig::ApiKey(cfg) => match db {
            Some(database) => Arc::new(ApiKeyAuthenticator::new(cfg.clone(), database)),
            None => {
                tracing::error!(
                    provider = %name,
                    "api_key provider requires a database; none configured — provider will always fail"
                );
                Arc::new(FailingAuthenticator::new(
                    "api_key provider requires a database connection".to_string(),
                ))
            }
        },
    }
}

/// Authenticator that always returns a provider error — used when a required
/// dependency (e.g. database) is missing at startup.
struct FailingAuthenticator {
    reason: String,
}

impl FailingAuthenticator {
    fn new(reason: String) -> Self {
        Self { reason }
    }
}

#[async_trait]
impl Authenticator for FailingAuthenticator {
    async fn authenticate(&self, _parts: &Parts) -> Result<AuthResult, AuthError> {
        Err(AuthError::ProviderError(self.reason.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn anonymous_always_succeeds() {
        let auth = AnonymousAuthenticator::new("guest");
        let (parts, _) = http::Request::new(()).into_parts();
        let result = auth.authenticate(&parts).await.unwrap();
        assert_eq!(result.identity.id, "guest");
        assert!(matches!(result.method, AuthMethod::Anonymous));
    }

    // ── C1/M3: MCP provider misconfiguration fails closed ──────────────────

    use crate::config::types::McpAuthConfig;

    fn mcp_provider(audience: Option<&str>, issuer: Option<&str>) -> AuthProviderConfig {
        AuthProviderConfig::Mcp(McpAuthConfig {
            jwks_url: "https://as.example/jwks".to_string(),
            issuer: issuer.map(str::to_string),
            audience: audience.map(str::to_string),
            resource: "https://rs.example/mcp".to_string(),
            authorization_servers: vec!["https://as.example".to_string()],
            required_scopes: vec![],
            leeway_seconds: 5,
        })
    }

    async fn build_single(name: &str, cfg: AuthProviderConfig) -> Arc<dyn Authenticator> {
        let mut providers = HashMap::new();
        providers.insert(name.to_string(), cfg);
        let map = build_authenticators(&providers, &reqwest::Client::new(), None);
        map.get(name).expect("provider built").clone()
    }

    #[tokio::test]
    async fn mcp_without_audience_yields_failing_authenticator() {
        // C1: no audience → RFC 8707 check would be a no-op → MUST fail closed.
        let auth = build_single("mcp", mcp_provider(None, Some("https://as.example"))).await;
        let (mut parts, _) = http::Request::new(()).into_parts();
        parts.headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer whatever"),
        );
        let result = auth.authenticate(&parts).await;
        assert!(
            matches!(result, Err(AuthError::ProviderError(_))),
            "MCP provider without audience must be a FailingAuthenticator"
        );
    }

    #[tokio::test]
    async fn mcp_without_issuer_yields_failing_authenticator() {
        // M3: no issuer → cannot pin trusted AS → fail closed.
        let auth = build_single("mcp", mcp_provider(Some("https://rs.example/mcp"), None)).await;
        let (parts, _) = http::Request::new(()).into_parts();
        let result = auth.authenticate(&parts).await;
        assert!(matches!(result, Err(AuthError::ProviderError(_))));
    }

    #[tokio::test]
    async fn mcp_with_audience_and_issuer_builds_real_authenticator() {
        // Sanity: a correctly configured provider does NOT fail at build; it
        // reaches the token path (missing Bearer → Unauthorized, not
        // ProviderError).
        let auth = build_single(
            "mcp",
            mcp_provider(Some("https://rs.example/mcp"), Some("https://as.example")),
        )
        .await;
        let (parts, _) = http::Request::new(()).into_parts();
        let result = auth.authenticate(&parts).await;
        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }
}
