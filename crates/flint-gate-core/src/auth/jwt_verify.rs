/// Inbound JWT Bearer verification authenticator.
///
/// Fetches the JWKS from the configured endpoint, caches it for 5 minutes,
/// verifies inbound `Authorization: Bearer <token>` requests, and maps the
/// JWT claims to an `Identity`.
use crate::auth::identity::Identity;
use crate::auth::jwks::JwksCache;
use crate::auth::{AuthError, AuthMethod, AuthResult, Authenticator};
use crate::config::types::JwtAuthConfig;
use async_trait::async_trait;
use http::header::AUTHORIZATION;
use http::request::Parts;
use jsonwebtoken::{decode, decode_header, Validation};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// JWT Bearer authenticator — verifies tokens against a JWKS endpoint.
pub struct JwtVerifyAuthenticator {
    config: JwtAuthConfig,
    /// The JWKS cache, or the construction error (invalid/SSRF `jwks_url`).
    /// Held as a `Result` so a bad URL fails CLOSED at authenticate time rather
    /// than panicking at build; the `new` signature stays infallible.
    jwks: Result<JwksCache, AuthError>,
}

impl JwtVerifyAuthenticator {
    pub fn new(config: JwtAuthConfig, client: reqwest::Client) -> Self {
        let jwks = JwksCache::new(config.jwks_url.clone(), client);
        Self { config, jwks }
    }

    /// Borrow the cache or reproduce the stored construction error.
    fn jwks(&self) -> Result<&JwksCache, AuthError> {
        self.jwks.as_ref().map_err(|e| match e {
            AuthError::ProviderError(m) => AuthError::ProviderError(m.clone()),
            other => AuthError::ProviderError(other.to_string()),
        })
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

        // ── Resolve decoding key from JWKS (shared cache + rotation) ───────
        let decoding_key = self.jwks()?.decoding_key(header.kid.as_deref()).await?;

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

        let (traits, metadata) = partition_jwt_claims(claims.rest);

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

/// Partition non-`sub` JWT claims into `(traits, metadata_public)`.
///
/// Well-known OIDC profile claims become identity traits; everything else is
/// metadata. Two classes are **dropped**:
/// - registered/temporal claims (`iss`, `iat`, `exp`, …) — non-identity;
/// - **`flint_kind`** — the gateway's own spoof-resistant principal-kind marker.
///   It is trusted ONLY on gateway-minted tokens; stripping it from every inbound
///   JWKS-verified token prevents an external IdP (or a self-service identity)
///   from forging `flint_kind: agent`/`service` to escalate to a non-human
///   principal. A legitimately-delegated agent re-enters via its RFC 8693 `act`
///   claim (see [`Identity::derived_kind`]), not a surviving `flint_kind`.
fn partition_jwt_claims(
    rest: std::collections::HashMap<String, Value>,
) -> (serde_json::Map<String, Value>, serde_json::Map<String, Value>) {
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
    const SKIP_KEYS: &[&str] = &[
        "iss",
        "iat",
        "exp",
        "nbf",
        "jti",
        "auth_time",
        crate::auth::identity::FLINT_KIND_CLAIM,
    ];

    let mut traits = serde_json::Map::new();
    let mut metadata = serde_json::Map::new();
    for (k, v) in rest {
        if SKIP_KEYS.contains(&k.as_str()) {
            continue;
        }
        if TRAIT_KEYS.contains(&k.as_str()) {
            traits.insert(k, v);
        } else {
            metadata.insert(k, v);
        }
    }
    (traits, metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::JwtAuthConfig;
    use std::collections::HashMap;

    #[test]
    fn partition_strips_inbound_flint_kind_spoof() {
        // A forged `flint_kind` on an externally-verified JWT must NOT survive
        // into metadata_public (else derived_kind would escalate it to Agent).
        let mut rest = HashMap::new();
        rest.insert("flint_kind".to_string(), serde_json::json!("agent"));
        rest.insert("act".to_string(), serde_json::json!({ "sub": "u" }));
        rest.insert("org".to_string(), serde_json::json!("acme"));
        rest.insert("email".to_string(), serde_json::json!("a@b.co"));
        let (traits, metadata) = partition_jwt_claims(rest);
        // flint_kind dropped; act + org kept as metadata; email routed to traits.
        assert!(!metadata.contains_key("flint_kind"), "flint_kind must be stripped");
        assert!(metadata.contains_key("act"));
        assert_eq!(metadata["org"], serde_json::json!("acme"));
        assert_eq!(traits["email"], serde_json::json!("a@b.co"));
        // And the resulting identity classifies as Agent via act, NOT flint_kind.
        let id = Identity {
            metadata_public: Value::Object(metadata),
            ..Default::default()
        };
        assert_eq!(id.derived_kind(), crate::auth::identity::IdentityKind::Agent);
    }

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
