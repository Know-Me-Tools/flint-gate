/// Authentication module — pluggable authenticator trait and built-in implementations.
pub mod identity;
pub mod jwt_mint;
pub mod kratos;

pub use identity::Identity;
pub use jwt_mint::{JwtMinter, SharedJwtMinter};
pub use kratos::KratosAuthenticator;

use crate::config::types::AuthProviderConfig;
use async_trait::async_trait;
use http::request::Parts;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

/// Error returned by an authenticator.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The credential is missing or invalid — return 401.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
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
    ApiKey { client_id: String, scopes: Vec<String> },
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
pub fn build_authenticators(
    providers: &HashMap<String, AuthProviderConfig>,
    http_client: &reqwest::Client,
) -> HashMap<String, Arc<dyn Authenticator>> {
    let mut map: HashMap<String, Arc<dyn Authenticator>> = HashMap::new();
    for (name, config) in providers {
        let auth: Arc<dyn Authenticator> = match config {
            AuthProviderConfig::Kratos(cfg) => {
                Arc::new(KratosAuthenticator::new(cfg.clone(), http_client.clone()))
            }
            AuthProviderConfig::Anonymous(cfg) => {
                Arc::new(AnonymousAuthenticator::new(cfg.default_subject.clone()))
            }
            AuthProviderConfig::Jwt(_cfg) => {
                // Phase 3: JWT verification authenticator
                tracing::warn!(provider = %name, "JWT authenticator not yet implemented; using anonymous fallback");
                Arc::new(AnonymousAuthenticator::new("jwt-stub"))
            }
            AuthProviderConfig::ApiKey(_cfg) => {
                // Phase 3: API key authenticator
                tracing::warn!(provider = %name, "API key authenticator not yet implemented; using anonymous fallback");
                Arc::new(AnonymousAuthenticator::new("api-key-stub"))
            }
        };
        map.insert(name.clone(), auth);
    }
    map
}

/// Thread-safe map of named authenticators.
pub type SharedAuthenticators = Arc<RwLock<HashMap<String, Arc<dyn Authenticator>>>>;

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
}
