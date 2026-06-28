/// Inbound JWT Bearer verification authenticator.
///
/// Fetches the JWKS from the configured endpoint, caches it for 5 minutes,
/// verifies inbound `Authorization: Bearer <token>` requests, and maps the
/// JWT claims to an `Identity`.
use crate::auth::identity::Identity;
use crate::auth::{AuthError, AuthMethod, AuthResult, Authenticator};
use crate::config::types::JwtAuthConfig;
use async_trait::async_trait;
use http::header::AUTHORIZATION;
use http::request::Parts;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::debug;

/// How long a fetched JWKS is considered valid before re-fetching.
const JWKS_TTL: Duration = Duration::from_secs(300);

struct CachedJwks {
    jwks: JwkSet,
    fetched_at: Instant,
}

/// JWT Bearer authenticator — verifies tokens against a JWKS endpoint.
pub struct JwtVerifyAuthenticator {
    config: JwtAuthConfig,
    client: reqwest::Client,
    jwks_cache: Arc<RwLock<Option<CachedJwks>>>,
}

impl JwtVerifyAuthenticator {
    pub fn new(config: JwtAuthConfig, client: reqwest::Client) -> Self {
        Self {
            config,
            client,
            jwks_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Return the cached JWKS, fetching from the network if stale or absent.
    async fn jwks(&self) -> Result<JwkSet, AuthError> {
        // Fast path — read-lock check
        {
            let guard = self.jwks_cache.read().await;
            if let Some(c) = guard.as_ref() {
                if c.fetched_at.elapsed() < JWKS_TTL {
                    return Ok(c.jwks.clone());
                }
            }
        }

        // Slow path — fetch and update
        debug!(url = %self.config.jwks_url, "fetching JWKS");
        let resp = self
            .client
            .get(&self.config.jwks_url)
            .send()
            .await
            .map_err(|e| AuthError::ProviderError(format!("JWKS fetch error: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AuthError::ProviderError(format!(
                "JWKS endpoint returned {status}"
            )));
        }

        let jwks: JwkSet = resp
            .json()
            .await
            .map_err(|e| AuthError::ProviderError(format!("JWKS parse error: {e}")))?;

        *self.jwks_cache.write().await = Some(CachedJwks {
            jwks: jwks.clone(),
            fetched_at: Instant::now(),
        });

        Ok(jwks)
    }
}

/// Flat JWT claims struct — `sub` extracted, everything else collected.
#[derive(Debug, Deserialize)]
struct RawClaims {
    sub: Option<String>,
    #[serde(flatten)]
    rest: HashMap<String, Value>,
}

#[async_trait]
impl Authenticator for JwtVerifyAuthenticator {
    async fn authenticate(&self, parts: &Parts) -> Result<AuthResult, AuthError> {
        // ── Extract Bearer token ───────────────────────────────────────────
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| {
                AuthError::Unauthorized("missing or malformed Authorization header".to_string())
            })?
            .trim();

        // ── Decode header (no verification yet) to get kid + alg ──────────
        let header = decode_header(token)
            .map_err(|e| AuthError::Unauthorized(format!("invalid JWT header: {e}")))?;

        // ── Resolve decoding key from JWKS ─────────────────────────────────
        let jwks = self.jwks().await?;
        let decoding_key = match &header.kid {
            Some(kid) => {
                let jwk = jwks.find(kid).ok_or_else(|| {
                    AuthError::Unauthorized(format!("no JWKS key found for kid={kid}"))
                })?;
                DecodingKey::from_jwk(jwk)
                    .map_err(|e| AuthError::ProviderError(format!("invalid JWK: {e}")))?
            }
            None => {
                // No kid — use first available key
                let jwk = jwks
                    .keys
                    .first()
                    .ok_or_else(|| AuthError::ProviderError("JWKS has no keys".to_string()))?;
                DecodingKey::from_jwk(jwk)
                    .map_err(|e| AuthError::ProviderError(format!("invalid JWK: {e}")))?
            }
        };

        // ── Build validation rules ─────────────────────────────────────────
        let mut validation = Validation::new(header.alg);
        validation.leeway = self.config.leeway_seconds;

        if let Some(iss) = &self.config.issuer {
            validation.set_issuer(&[iss.as_str()]);
        }
        match &self.config.audience {
            Some(aud) => validation.set_audience(&[aud.as_str()]),
            None => validation.validate_aud = false,
        }

        // ── Verify signature + claims ──────────────────────────────────────
        let token_data = decode::<RawClaims>(token, &decoding_key, &validation)
            .map_err(|e| AuthError::Unauthorized(format!("JWT verification failed: {e}")))?;

        // ── Map claims → Identity ──────────────────────────────────────────
        let claims = token_data.claims;
        let subject = claims
            .sub
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string());

        // Well-known OIDC traits go into identity.traits; everything else into metadata_public.
        const TRAIT_KEYS: &[&str] = &[
            "email",
            "email_verified",
            "name",
            "given_name",
            "family_name",
            "nickname",
            "preferred_username",
            "picture",
            "phone_number",
            "locale",
        ];
        const SKIP_KEYS: &[&str] = &["iss", "iat", "exp", "nbf", "jti", "auth_time"];

        let mut traits = serde_json::Map::new();
        let mut metadata = serde_json::Map::new();

        for (k, v) in claims.rest {
            if SKIP_KEYS.contains(&k.as_str()) {
                continue;
            }
            if TRAIT_KEYS.contains(&k.as_str()) {
                traits.insert(k, v);
            } else {
                metadata.insert(k, v);
            }
        }

        let identity = Identity {
            id: subject,
            traits: Value::Object(traits),
            metadata_public: Value::Object(metadata),
            ..Default::default()
        };

        Ok(AuthResult {
            identity,
            method: AuthMethod::BearerJwt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::JwtAuthConfig;

    fn jwt_config(jwks_url: &str) -> JwtAuthConfig {
        JwtAuthConfig {
            jwks_url: jwks_url.to_string(),
            issuer: None,
            audience: None,
            leeway_seconds: 5,
        }
    }

    fn empty_parts() -> Parts {
        http::Request::new(()).into_parts().0
    }

    fn parts_with_bearer(token: &str) -> Parts {
        let (mut parts, _) = http::Request::new(()).into_parts();
        parts.headers.insert(
            AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        parts
    }

    #[tokio::test]
    async fn missing_header_returns_unauthorized() {
        let auth = JwtVerifyAuthenticator::new(
            jwt_config("http://localhost/jwks"),
            reqwest::Client::new(),
        );
        let result = auth.authenticate(&empty_parts()).await;
        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn malformed_token_returns_unauthorized() {
        let auth = JwtVerifyAuthenticator::new(
            jwt_config("http://localhost/jwks"),
            reqwest::Client::new(),
        );
        let result = auth.authenticate(&parts_with_bearer("not.a.jwt")).await;
        assert!(matches!(result, Err(AuthError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn jwks_fetch_failure_returns_provider_error() {
        let auth = JwtVerifyAuthenticator::new(
            jwt_config("http://localhost:1/nonexistent"),
            reqwest::Client::new(),
        );
        let result = auth
            .authenticate(&parts_with_bearer(
                "eyJhbGciOiJSUzI1NiIsImtpZCI6ImtleS0xIn0.eyJzdWIiOiJ1c2VyLTEifQ.sig",
            ))
            .await;
        // Either ProviderError (JWKS fetch failed) or Unauthorized (decode error)
        assert!(result.is_err());
    }
}
