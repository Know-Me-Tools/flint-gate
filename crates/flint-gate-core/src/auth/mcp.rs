//! MCP-era OAuth 2.1 **Resource Server** authenticator.
//!
//! flint-gate here is a Resource Server (RS), never an Authorization Server. It
//! validates access tokens minted by a trusted external AS and enforces the
//! bindings the MCP profile requires on top of a plain JWT check:
//!
//! 1. Signature + standard claims (`exp`/`nbf`/`iss`) — via `jsonwebtoken@9`,
//!    mirroring [`crate::auth::jwt_verify`].
//! 2. **RFC 8707 audience binding (security crux)** — the token's `aud` MUST
//!    include this RS's configured `audience`/resource. A token minted for a
//!    *different* resource is rejected even if its signature is valid. This is
//!    the confused-deputy defense: without it, a token issued to some other RS
//!    could be replayed here.
//! 3. **Scope gate** — the token's granted scopes (`scope` space-delimited
//!    string or `scp` array) MUST be a superset of `required_scopes`, else 403
//!    `insufficient_scope`.
//!
//! Every decision fails CLOSED: unknown/ambiguous state → deny. No `unwrap()` on
//! the auth path.

use crate::auth::identity::Identity;
use crate::auth::jwks::JwksCache;
use crate::auth::{AuthError, AuthMethod, AuthResult, Authenticator};
use crate::config::types::McpAuthConfig;
use async_trait::async_trait;
use http::header::AUTHORIZATION;
use http::request::Parts;
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tracing::debug;

/// Asymmetric-only algorithm allowlist (M1).
///
/// The RS pins the set of acceptable signature algorithms and refuses anything
/// else BEFORE resolving a key. This, combined with symmetric-JWK rejection in
/// [`crate::auth::jwks`], closes the alg-confusion / `alg:none` / HMAC-downgrade
/// class: a token cannot dictate a symmetric or unexpected algorithm.
const ALLOWED_ALGS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::ES256,
    Algorithm::ES384,
];

/// MCP OAuth 2.1 Resource Server authenticator.
pub struct McpAuthenticator {
    config: McpAuthConfig,
    /// JWKS cache or its construction error (invalid/SSRF `jwks_url`). Held as a
    /// `Result` so a bad URL fails CLOSED at authenticate time.
    jwks: Result<JwksCache, AuthError>,
}

impl McpAuthenticator {
    pub fn new(config: McpAuthConfig, client: reqwest::Client) -> Self {
        let jwks = JwksCache::new(config.jwks_url.clone(), client);
        Self { config, jwks }
    }

    fn jwks(&self) -> Result<&JwksCache, AuthError> {
        self.jwks
            .as_ref()
            .map_err(|e| AuthError::ProviderError(e.to_string()))
    }
}

/// Raw claims: `sub`, the RFC 8707 audience, and the two scope shapes are pulled
/// out explicitly; everything else is collected for the identity mapping.
#[derive(Debug, Deserialize)]
struct McpClaims {
    sub: Option<String>,
    /// `aud` may be a single string or an array of strings per RFC 7519.
    #[serde(default)]
    aud: Audience,
    /// Space-delimited scope string (RFC 8693 / OAuth token responses).
    #[serde(default)]
    scope: Option<String>,
    /// Array-of-scopes form (some ASs emit `scp`).
    #[serde(default)]
    scp: Option<Vec<String>>,
    #[serde(flatten)]
    rest: HashMap<String, Value>,
}

/// `aud` is either a single string or a list of strings.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Audience {
    #[default]
    Absent,
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, target: &str) -> bool {
        match self {
            Audience::Absent => false,
            Audience::One(a) => a == target,
            Audience::Many(list) => list.iter().any(|a| a == target),
        }
    }
}

/// Parse granted scopes from the two accepted claim shapes into a set-like Vec.
/// `scope` (space-delimited) and `scp` (array) are unioned; empty entries are
/// dropped.
fn granted_scopes(scope: &Option<String>, scp: &Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(s) = scope {
        out.extend(s.split_whitespace().map(str::to_string));
    }
    if let Some(list) = scp {
        out.extend(list.iter().filter(|s| !s.is_empty()).cloned());
    }
    out
}

/// True iff `granted` is a superset of every entry in `required`.
///
/// Empty `required` ⇒ vacuously true (no scope gate configured). This is a pure
/// function so it is unit-testable without any network or token machinery.
fn scopes_sufficient(granted: &[String], required: &[String]) -> bool {
    required.iter().all(|r| granted.iter().any(|g| g == r))
}

/// Map decoded claims into an [`Identity`], mirroring `jwt_verify`'s trait/metadata
/// split so downstream template contexts see a consistent shape regardless of
/// which authenticator ran.
fn identity_from_claims(claims: McpClaims) -> (Identity, Vec<String>) {
    let subject = claims
        .sub
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let scopes = granted_scopes(&claims.scope, &claims.scp);

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
    // `flint_kind` is STRIPPED: it is the gateway's own principal-kind marker and
    // must never be trusted from a JWKS-federated token (an external IdP could
    // set it to escalate to Agent/Service). Delegated agents re-enter via `act`.
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

    let mut extra = HashMap::new();
    if !scopes.is_empty() {
        // Expose granted scopes to the template/streaming layer, matching the
        // `a2ui_scopes` convention the pipeline already reads.
        extra.insert("mcp_scopes".to_string(), scopes.join(" "));
    }

    let identity = Identity {
        id: subject,
        traits: Value::Object(traits),
        metadata_public: Value::Object(metadata),
        extra,
        ..Default::default()
    };
    (identity, scopes)
}

#[async_trait]
impl Authenticator for McpAuthenticator {
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

        // ── Header → kid/alg (no verification yet) ─────────────────────────
        let header = decode_header(token)
            .map_err(|e| AuthError::Unauthorized(format!("invalid JWT header: {e}")))?;

        // ── Algorithm allowlist (M1) — reject BEFORE key resolution ────────
        // Refuse `alg:none`, HS*, and anything outside the asymmetric allowlist
        // up front, so a hostile token can never steer us toward symmetric
        // verification or trigger a key lookup for a disallowed algorithm.
        if !ALLOWED_ALGS.contains(&header.alg) {
            return Err(AuthError::Unauthorized(format!(
                "token algorithm {:?} not permitted",
                header.alg
            )));
        }

        // ── Resolve signing key (shared cache + rotation) ──────────────────
        let decoding_key = self.jwks()?.decoding_key(header.kid.as_deref()).await?;

        // ── Validation rules ───────────────────────────────────────────────
        // Pin `validation.algorithms` to exactly the token's header alg. It has
        // already passed the `ALLOWED_ALGS` allowlist above, so this is safe and
        // is defense-in-depth. NOTE: we must NOT stuff the whole mixed-family
        // allowlist (RSA + EC) into `validation.algorithms` — jsonwebtoken@9
        // rejects a verify whose algorithm list spans a different key family
        // than the resolved key (`InvalidAlgorithm`). The up-front allowlist
        // check is the actual gate; this just scopes the verifier to one alg.
        //
        // We do NOT delegate audience validation to jsonwebtoken here: RFC 8707
        // requires that the RS's resource be *present in* the token audience,
        // and we want a distinct, auditable rejection at the exact enforcement
        // point below — so we disable the library's aud check and enforce it
        // ourselves after decode.
        let mut validation = Validation::new(header.alg);
        validation.algorithms = vec![header.alg];
        validation.leeway = self.config.leeway_seconds;
        validation.validate_aud = false;
        if let Some(iss) = &self.config.issuer {
            validation.set_issuer(&[iss.as_str()]);
        }

        // ── Verify signature + standard claims ─────────────────────────────
        let token_data = decode::<McpClaims>(token, &decoding_key, &validation)
            .map_err(|e| AuthError::Unauthorized(format!("JWT verification failed: {e}")))?;
        let claims = token_data.claims;

        // ── RFC 8707 audience binding — THE confused-deputy defense ─────────
        // If this RS declares a resource identifier, the token's `aud` MUST
        // include it. Reject otherwise, even though the signature verified.
        if let Some(expected_aud) = &self.config.audience {
            if !claims.aud.contains(expected_aud) {
                debug!(
                    expected = %expected_aud,
                    "token audience does not include this resource — rejecting (RFC 8707)"
                );
                return Err(AuthError::Unauthorized(format!(
                    "token audience does not include resource {expected_aud}"
                )));
            }
        }

        // ── Scope gate (403 insufficient_scope on failure) ─────────────────
        let (identity, granted) = identity_from_claims(claims);
        if !scopes_sufficient(&granted, &self.config.required_scopes) {
            return Err(AuthError::InsufficientScope {
                required: self.config.required_scopes.clone(),
            });
        }

        Ok(AuthResult {
            identity,
            method: AuthMethod::McpBearer { scopes: granted },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RFC 8707 audience accept / reject ──────────────────────────────────

    #[test]
    fn audience_single_match_accepts() {
        let aud = Audience::One("https://rs.example/mcp".to_string());
        assert!(aud.contains("https://rs.example/mcp"));
    }

    #[test]
    fn audience_single_mismatch_rejects() {
        let aud = Audience::One("https://other.example/mcp".to_string());
        assert!(!aud.contains("https://rs.example/mcp"));
    }

    #[test]
    fn audience_array_containing_resource_accepts() {
        let aud = Audience::Many(vec![
            "https://other.example".to_string(),
            "https://rs.example/mcp".to_string(),
        ]);
        assert!(aud.contains("https://rs.example/mcp"));
    }

    #[test]
    fn audience_array_without_resource_rejects() {
        let aud = Audience::Many(vec!["https://other.example".to_string()]);
        assert!(!aud.contains("https://rs.example/mcp"));
    }

    #[test]
    fn audience_absent_rejects() {
        // Fail-closed: a token with no audience cannot satisfy an RFC 8707 bind.
        assert!(!Audience::Absent.contains("https://rs.example/mcp"));
    }

    // ── Scope sufficiency ──────────────────────────────────────────────────

    #[test]
    fn scopes_superset_is_sufficient() {
        let granted = vec!["read".to_string(), "write".to_string(), "admin".to_string()];
        let required = vec!["read".to_string(), "write".to_string()];
        assert!(scopes_sufficient(&granted, &required));
    }

    #[test]
    fn scopes_missing_one_is_insufficient() {
        let granted = vec!["read".to_string()];
        let required = vec!["read".to_string(), "write".to_string()];
        assert!(!scopes_sufficient(&granted, &required));
    }

    #[test]
    fn empty_required_scopes_always_sufficient() {
        assert!(scopes_sufficient(&[], &[]));
        assert!(scopes_sufficient(&["read".to_string()], &[]));
    }

    // ── Scope parsing from the two claim shapes ────────────────────────────

    #[test]
    fn granted_scopes_parses_space_delimited() {
        let scopes = granted_scopes(&Some("read write  admin".to_string()), &None);
        assert_eq!(scopes, vec!["read", "write", "admin"]);
    }

    #[test]
    fn granted_scopes_parses_scp_array_and_unions() {
        let scopes = granted_scopes(
            &Some("read".to_string()),
            &Some(vec!["write".to_string(), "".to_string()]),
        );
        assert!(scopes.contains(&"read".to_string()));
        assert!(scopes.contains(&"write".to_string()));
        assert!(!scopes.contains(&"".to_string()), "empty scope dropped");
    }

    #[test]
    fn identity_from_claims_maps_sub_traits_and_scopes() {
        let claims = McpClaims {
            sub: Some("agent-1".to_string()),
            aud: Audience::One("https://rs.example/mcp".to_string()),
            scope: Some("read write".to_string()),
            scp: None,
            rest: HashMap::from([
                ("email".to_string(), Value::String("a@b.com".to_string())),
                ("org".to_string(), Value::String("acme".to_string())),
            ]),
        };
        let (identity, scopes) = identity_from_claims(claims);
        assert_eq!(identity.id, "agent-1");
        assert_eq!(identity.traits["email"], "a@b.com");
        assert_eq!(identity.metadata_public["org"], "acme");
        assert_eq!(
            identity.extra.get("mcp_scopes").map(String::as_str),
            Some("read write")
        );
        assert_eq!(scopes, vec!["read", "write"]);
    }

    fn test_cfg() -> McpAuthConfig {
        McpAuthConfig {
            jwks_url: "http://127.0.0.1:1/jwks".to_string(),
            issuer: None,
            audience: Some("https://rs.example/mcp".to_string()),
            resource: "https://rs.example/mcp".to_string(),
            authorization_servers: vec![],
            required_scopes: vec![],
            leeway_seconds: 5,
        }
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
    async fn missing_bearer_is_unauthorized() {
        let auth = McpAuthenticator::new(test_cfg(), reqwest::Client::new());
        let (parts, _) = http::Request::new(()).into_parts();
        assert!(matches!(
            auth.authenticate(&parts).await,
            Err(AuthError::Unauthorized(_))
        ));
    }

    // ── M1: algorithm allowlist — HS256 rejected before key resolution ──────

    #[tokio::test]
    async fn hs256_token_is_rejected_before_network() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        // A real HS256 token. `jwks_url` points at an unbound port; if the alg
        // check did NOT fire first, we'd get a ProviderError (fetch failure).
        // We assert Unauthorized (alg rejected up front) instead.
        let header = Header::new(jsonwebtoken::Algorithm::HS256);
        let claims = serde_json::json!({
            "sub": "attacker",
            "aud": "https://rs.example/mcp",
        });
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_secret(b"shared-secret"),
        )
        .unwrap();

        let auth = McpAuthenticator::new(test_cfg(), reqwest::Client::new());
        let result = auth.authenticate(&parts_with_bearer(&token)).await;
        match result {
            Err(AuthError::Unauthorized(msg)) => {
                assert!(
                    msg.contains("not permitted"),
                    "expected alg rejection, got: {msg}"
                );
            }
            other => panic!("expected Unauthorized alg rejection, got {other:?}"),
        }
    }

    #[test]
    fn allowed_algs_are_asymmetric_only() {
        // Guard: the allowlist must never contain a symmetric algorithm.
        for alg in ALLOWED_ALGS {
            assert!(
                !matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512),
                "symmetric algorithm leaked into allowlist: {alg:?}"
            );
        }
    }
}
