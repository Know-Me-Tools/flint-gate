/// Ory Kratos session authenticator.
///
/// Calls Kratos `GET /sessions/whoami`, forwarding the session cookie or
/// `Authorization: Bearer` header from the incoming request. Extracts the
/// full identity into our universal [`Identity`] struct.
use crate::auth::identity::Identity;
use crate::auth::{AuthError, AuthResult, Authenticator};
use crate::config::types::KratosAuthConfig;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http::header::{AUTHORIZATION, COOKIE};
use http::request::Parts;
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

/// Kratos session authenticator.
pub struct KratosAuthenticator {
    config: KratosAuthConfig,
    client: reqwest::Client,
    cache_context: Option<KratosCacheContext>,
}

#[derive(Debug, Clone)]
pub struct KratosCacheContext {
    issuer: Option<String>,
    forward_cookies: bool,
    session_cookie: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KratosCredentialKind {
    Authorization,
    Cookie,
}

#[derive(Debug, Clone)]
pub struct KratosCredential {
    pub kind: KratosCredentialKind,
    pub cache_value: Vec<u8>,
    header_value: HeaderValue,
}

impl KratosCacheContext {
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }

    pub fn select_credential(&self, headers: &HeaderMap) -> Result<KratosCredential, AuthError> {
        select_credential(headers, self.forward_cookies, self.session_cookie.as_str())
    }
}

impl KratosCredential {
    pub fn insert_header(&self, headers: &mut HeaderMap) {
        match self.kind {
            KratosCredentialKind::Authorization => {
                headers.insert(AUTHORIZATION, self.header_value.clone());
            }
            KratosCredentialKind::Cookie => {
                headers.insert(COOKIE, self.header_value.clone());
            }
        }
    }
}

impl KratosAuthenticator {
    /// Create a new Kratos authenticator from config.
    pub fn new(config: KratosAuthConfig, client: reqwest::Client) -> Self {
        let cache_context = Some(KratosCacheContext {
            issuer: config.issuer.as_deref().and_then(canonical_issuer),
            forward_cookies: config.forward_cookies,
            session_cookie: config.session_cookie.clone(),
        });
        Self {
            config,
            client,
            cache_context,
        }
    }
}

fn canonical_issuer(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.to_string())
}

fn select_credential(
    headers: &HeaderMap,
    forward_cookies: bool,
    session_cookie: &str,
) -> Result<KratosCredential, AuthError> {
    let authorization_values: Vec<_> = headers.get_all(AUTHORIZATION).iter().collect();
    let cookie_values: Vec<_> = headers.get_all(COOKIE).iter().collect();
    if authorization_values.len() > 1 || cookie_values.len() > 1 {
        return Err(AuthError::Unauthorized(
            "multiple Kratos credential headers are not allowed".to_string(),
        ));
    }

    let authorization = authorization_values.first().copied();
    let session_cookie_value = if forward_cookies {
        cookie_values
            .first()
            .copied()
            .map(|header| extract_cookie(header, session_cookie))
            .transpose()?
            .flatten()
    } else {
        None
    };
    if authorization.is_some() && session_cookie_value.is_some() {
        return Err(AuthError::Unauthorized(
            "competing Kratos credentials are not allowed".to_string(),
        ));
    }
    if let Some(value) = authorization {
        if value.to_str().ok().is_none_or(|raw| raw.trim().is_empty()) {
            return Err(AuthError::Unauthorized(
                "Kratos authorization credential is blank".to_string(),
            ));
        }
        return Ok(KratosCredential {
            kind: KratosCredentialKind::Authorization,
            cache_value: value.as_bytes().to_vec(),
            header_value: value.clone(),
        });
    }
    if let Some((header, value)) = session_cookie_value {
        return Ok(KratosCredential {
            kind: KratosCredentialKind::Cookie,
            cache_value: value.into_bytes(),
            header_value: header,
        });
    }
    Err(AuthError::Unauthorized(
        "Kratos session credential is missing".to_string(),
    ))
}

fn extract_cookie(
    header: &HeaderValue,
    session_cookie: &str,
) -> Result<Option<(HeaderValue, String)>, AuthError> {
    let raw = header
        .to_str()
        .map_err(|_| AuthError::Unauthorized("Kratos cookie header is invalid".to_string()))?;
    let mut selected = None;
    for cookie in raw.split(';') {
        let Some((name, value)) = cookie.trim().split_once('=') else {
            continue;
        };
        if name.trim() == session_cookie {
            if selected.is_some() || value.trim().is_empty() {
                return Err(AuthError::Unauthorized(
                    "Kratos session cookie is ambiguous or blank".to_string(),
                ));
            }
            selected = Some((header.clone(), value.trim().to_string()));
        }
    }
    Ok(selected)
}

/// Subset of the Kratos `/sessions/whoami` response we care about.
#[derive(Debug, Deserialize)]
struct KratosSession {
    id: Option<String>,
    active: bool,
    expires_at: DateTime<Utc>,
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
        let credential = select_credential(
            &parts.headers,
            self.config.forward_cookies,
            self.config.session_cookie.as_str(),
        )?;
        let mut req = self.client.get(format!(
            "{}/sessions/whoami",
            self.config.base_url.trim_end_matches('/')
        ));

        match credential.kind {
            KratosCredentialKind::Authorization => {
                req = req.header(AUTHORIZATION, credential.header_value);
            }
            KratosCredentialKind::Cookie => {
                req = req.header(COOKIE, credential.header_value);
            }
        }

        let response = req.send().await.map_err(|e| {
            tracing::warn!(error = %e, "Kratos request failed");
            AuthError::ProviderError(format!("Kratos unreachable: {e}"))
        })?;

        let status = response.status();

        if status == http::StatusCode::UNAUTHORIZED || status == http::StatusCode::FORBIDDEN {
            return Err(AuthError::Unauthorized(
                "invalid or expired Kratos session".to_string(),
            ));
        }

        if !status.is_success() {
            return Err(AuthError::ProviderError(format!(
                "Kratos returned status {status}"
            )));
        }

        let session: KratosSession = response.json().await.map_err(|e| {
            AuthError::ProviderError(format!("failed to parse Kratos response: {e}"))
        })?;

        if !session.active || session.expires_at <= Utc::now() {
            return Err(AuthError::Unauthorized(
                "Kratos session is inactive".to_string(),
            ));
        }

        let verified_session_id =
            session
                .id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    AuthError::ProviderError(
                        "Kratos response missing verified session ID".to_string(),
                    )
                })?;
        let kratos_id = session.identity.ok_or_else(|| {
            AuthError::ProviderError("Kratos response missing identity".to_string())
        })?;

        // Kratos `metadata_public` is admin- and (in some deployments)
        // self-service-writable, so it is UNTRUSTED for principal-kind: strip any
        // `flint_kind` a Kratos identity might carry, otherwise a human could set
        // `flint_kind: service` and escalate to a non-human principal.
        let mut metadata_public = kratos_id.metadata_public.unwrap_or(Value::Null);
        crate::auth::identity::strip_untrusted_kind(&mut metadata_public);
        let identity = Identity {
            id: kratos_id.id,
            // Kratos authenticates human sessions.
            kind: crate::auth::identity::IdentityKind::User,
            traits: kratos_id.traits.unwrap_or(Value::Null),
            metadata_public,
            schema_id: kratos_id.schema_id,
            session_id: Some(verified_session_id),
            session_expires_at: Some(session.expires_at),
            aal: session.authenticator_assurance_level,
            extra: Default::default(),
        };

        Ok(AuthResult {
            identity,
            method: crate::auth::AuthMethod::KratosSession,
        })
    }

    fn kratos_cache_context(&self) -> Option<&KratosCacheContext> {
        self.cache_context.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::KratosAuthConfig;

    fn default_config(base_url: &str) -> KratosAuthConfig {
        KratosAuthConfig {
            base_url: base_url.to_string(),
            issuer: Some("https://identity.example.test".to_string()),
            forward_cookies: true,
            session_cookie: "ory_kratos_session".to_string(),
        }
    }

    fn empty_parts() -> Parts {
        let (parts, _) = http::Request::builder()
            .header(COOKIE, "ory_kratos_session=test-session")
            .body(())
            .unwrap()
            .into_parts();
        parts
    }

    #[tokio::test]
    async fn returns_unauthorized_on_401() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sessions/whoami"))
            .respond_with(wiremock::ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let auth = KratosAuthenticator::new(default_config(&server.uri()), reqwest::Client::new());
        let result = auth.authenticate(&empty_parts()).await;
        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn retains_the_authoritative_session_expiry() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sessions/whoami"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "session-1",
                    "active": true,
                    "expires_at": "2099-01-01T00:00:00Z",
                    "identity": { "id": "identity-1" }
                })),
            )
            .mount(&server)
            .await;

        let auth = KratosAuthenticator::new(default_config(&server.uri()), reqwest::Client::new());
        let result = auth
            .authenticate(&empty_parts())
            .await
            .expect("valid session");

        assert_eq!(result.identity.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            result.identity.session_expires_at,
            Some("2099-01-01T00:00:00Z".parse().expect("fixed timestamp"))
        );
    }

    #[tokio::test]
    async fn refuses_an_expired_success_response() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sessions/whoami"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "session-1",
                    "active": true,
                    "expires_at": "2000-01-01T00:00:00Z",
                    "identity": { "id": "identity-1" }
                })),
            )
            .mount(&server)
            .await;

        let auth = KratosAuthenticator::new(default_config(&server.uri()), reqwest::Client::new());
        let result = auth.authenticate(&empty_parts()).await;

        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }

    #[test]
    fn credential_selection_rejects_competing_session_sources() {
        let config = default_config("https://kratos.example.test");
        let auth = KratosAuthenticator::new(config, reqwest::Client::new());
        let (parts, _) = http::Request::builder()
            .header(AUTHORIZATION, "Bearer token-a")
            .header(COOKIE, "other=1; ory_kratos_session=session-b")
            .body(())
            .unwrap()
            .into_parts();

        let result = auth
            .kratos_cache_context()
            .unwrap()
            .select_credential(&parts.headers);

        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }

    #[test]
    fn cookie_cache_key_uses_only_the_configured_session_cookie_value() {
        let config = default_config("https://kratos.example.test");
        let auth = KratosAuthenticator::new(config, reqwest::Client::new());
        let (parts, _) = http::Request::builder()
            .header(
                COOKIE,
                "theme=warm; ory_kratos_session=session-a; locale=en",
            )
            .body(())
            .unwrap()
            .into_parts();

        let credential = auth
            .kratos_cache_context()
            .unwrap()
            .select_credential(&parts.headers)
            .unwrap();

        assert_eq!(credential.kind, KratosCredentialKind::Cookie);
        assert_eq!(credential.cache_value, b"session-a");
    }

    #[test]
    fn authority_cache_requires_an_explicit_canonical_issuer() {
        let explicit = KratosAuthenticator::new(
            default_config("https://kratos.example.test"),
            reqwest::Client::new(),
        );
        assert_eq!(
            explicit.kratos_cache_context().unwrap().issuer(),
            Some("https://identity.example.test/")
        );

        let mut missing = default_config("https://kratos.example.test");
        missing.issuer = None;
        let missing = KratosAuthenticator::new(missing, reqwest::Client::new());
        assert_eq!(missing.kratos_cache_context().unwrap().issuer(), None);

        let mut invalid = default_config("https://kratos.example.test");
        invalid.issuer = Some("https://user@identity.example.test/?query=1".to_string());
        let invalid = KratosAuthenticator::new(invalid, reqwest::Client::new());
        assert_eq!(invalid.kratos_cache_context().unwrap().issuer(), None);
    }

    #[tokio::test]
    async fn refuses_success_without_a_verified_session_id() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sessions/whoami"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "active": true,
                    "expires_at": "2099-01-01T00:00:00Z",
                    "identity": { "id": "identity-1" }
                })),
            )
            .mount(&server)
            .await;
        let auth = KratosAuthenticator::new(default_config(&server.uri()), reqwest::Client::new());

        let result = auth.authenticate(&empty_parts()).await;

        assert!(matches!(result, Err(AuthError::ProviderError(_))));
    }
}
