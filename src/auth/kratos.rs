/// Ory Kratos session authenticator.
///
/// Calls Kratos `GET /sessions/whoami`, forwarding the session cookie or
/// `Authorization: Bearer` header from the incoming request. Extracts the
/// full identity into our universal [`Identity`] struct.
use crate::auth::identity::Identity;
use crate::auth::{AuthError, AuthResult, Authenticator};
use crate::config::types::KratosAuthConfig;
use async_trait::async_trait;
use http::header::{AUTHORIZATION, COOKIE};
use http::request::Parts;
use serde::Deserialize;
use serde_json::Value;

/// Kratos session authenticator.
pub struct KratosAuthenticator {
    config: KratosAuthConfig,
    client: reqwest::Client,
}

impl KratosAuthenticator {
    /// Create a new Kratos authenticator from config.
    pub fn new(config: KratosAuthConfig, client: reqwest::Client) -> Self {
        Self { config, client }
    }
}

/// Subset of the Kratos `/sessions/whoami` response we care about.
#[derive(Debug, Deserialize)]
struct KratosSession {
    id: Option<String>,
    active: Option<bool>,
    identity: Option<KratosIdentity>,
    authenticator_assurance_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KratosIdentity {
    id: String,
    traits: Option<Value>,
    metadata_public: Option<Value>,
    schema_id: Option<String>,
}

#[async_trait]
impl Authenticator for KratosAuthenticator {
    async fn authenticate(&self, parts: &Parts) -> Result<AuthResult, AuthError> {
        let mut req = self
            .client
            .get(format!("{}/sessions/whoami", self.config.base_url.trim_end_matches('/')));

        // Forward session cookie if present
        if self.config.forward_cookies {
            if let Some(cookie_header) = parts.headers.get(COOKIE) {
                req = req.header(COOKIE, cookie_header);
            }
        }

        // Forward Authorization header if present
        if let Some(auth_header) = parts.headers.get(AUTHORIZATION) {
            req = req.header(AUTHORIZATION, auth_header);
        }

        let response = req.send().await.map_err(|e| {
            tracing::warn!(error = %e, "Kratos request failed");
            AuthError::ProviderError(format!("Kratos unreachable: {e}"))
        })?;

        let status = response.status();

        if status == http::StatusCode::UNAUTHORIZED || status == http::StatusCode::FORBIDDEN {
            return Err(AuthError::Unauthorized("invalid or expired Kratos session".to_string()));
        }

        if !status.is_success() {
            return Err(AuthError::ProviderError(format!(
                "Kratos returned status {status}"
            )));
        }

        let session: KratosSession = response.json().await.map_err(|e| {
            AuthError::ProviderError(format!("failed to parse Kratos response: {e}"))
        })?;

        if session.active == Some(false) {
            return Err(AuthError::Unauthorized("Kratos session is inactive".to_string()));
        }

        let kratos_id = session.identity.ok_or_else(|| {
            AuthError::ProviderError("Kratos response missing identity".to_string())
        })?;

        let identity = Identity {
            id: kratos_id.id,
            traits: kratos_id.traits.unwrap_or(Value::Null),
            metadata_public: kratos_id.metadata_public.unwrap_or(Value::Null),
            schema_id: kratos_id.schema_id,
            session_id: session.id,
            aal: session.authenticator_assurance_level,
            extra: Default::default(),
        };

        Ok(AuthResult {
            identity,
            method: crate::auth::AuthMethod::KratosSession,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::KratosAuthConfig;
    use http::request::Builder;

    fn default_config(base_url: &str) -> KratosAuthConfig {
        KratosAuthConfig {
            base_url: base_url.to_string(),
            forward_cookies: true,
            session_cookie: "ory_kratos_session".to_string(),
        }
    }

    fn empty_parts() -> Parts {
        let (parts, _) = http::Request::new(()).into_parts();
        parts
    }

    #[tokio::test]
    async fn returns_unauthorized_on_401() {
        let mut server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sessions/whoami"))
            .respond_with(wiremock::ResponseTemplate::new(401))
            .mount(&mut server)
            .await;

        let auth = KratosAuthenticator::new(
            default_config(&server.uri()),
            reqwest::Client::new(),
        );
        let result = auth.authenticate(&empty_parts()).await;
        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }
}
