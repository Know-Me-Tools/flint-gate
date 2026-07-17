//! OAuth 2.0 Token Introspection (RFC 7662) for **gateway-minted** tokens.
//!
//! `POST /oauth/introspect` verifies a presented token against the gateway's own
//! signing material (the same `jwt` config the minter uses) and returns the
//! RFC 7662 response. An unknown / expired / invalid / wrong-issuer token yields
//! `{"active": false}` — never an error that leaks whether the token merely
//! failed a specific check (RFC 7662 §2.2).
//!
//! Where Ory Hydra is the authorization server, Hydra owns introspection for its
//! own opaque tokens; the `introspection_delegate` seam (config) is for
//! forwarding those to Hydra and is defined separately.

use crate::config::types::JwtConfig;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The parsed introspection request (`token` form field; `token_type_hint`
/// ignored — we only introspect access tokens we minted).
#[derive(Debug, Deserialize)]
pub struct IntrospectRequest {
    pub token: String,
    #[serde(default)]
    pub token_type_hint: Option<String>,
}

/// A ready-to-use verifier for gateway-minted tokens, built once from the
/// signing config. Cheap to clone (DecodingKey is reference-counted internally
/// via owned bytes).
#[derive(Clone)]
pub struct TokenVerifier {
    decoding_key: DecodingKey,
    validation: Validation,
}

impl TokenVerifier {
    /// Build a verifier from the gateway's `jwt` signing config. Supports the
    /// symmetric HS* algorithms (verify with the shared secret) and asymmetric
    /// RS*/ES* (verify with the public component of the configured PEM).
    ///
    /// Returns `Err` when the config lacks the material needed to verify
    /// (e.g. HS* without a secret) — the caller then disables introspection
    /// rather than accepting tokens it cannot check.
    pub async fn from_jwt_config(cfg: &JwtConfig) -> anyhow::Result<Self> {
        use anyhow::Context;
        let (alg, decoding_key) = match cfg.signing_algorithm.as_str() {
            "HS256" | "HS384" | "HS512" => {
                let secret = cfg
                    .signing_key_secret
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .context("HS* introspection requires signing_key_secret")?;
                let alg = match cfg.signing_algorithm.as_str() {
                    "HS384" => Algorithm::HS384,
                    "HS512" => Algorithm::HS512,
                    _ => Algorithm::HS256,
                };
                (alg, DecodingKey::from_secret(secret.as_bytes()))
            }
            "RS256" | "RS384" | "RS512" => {
                let path = cfg
                    .signing_key_path
                    .as_deref()
                    .context("RS* introspection requires signing_key_path")?;
                let pem = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("reading RSA key from {path}"))?;
                let alg = match cfg.signing_algorithm.as_str() {
                    "RS384" => Algorithm::RS384,
                    "RS512" => Algorithm::RS512,
                    _ => Algorithm::RS256,
                };
                // jsonwebtoken derives the public key from an RSA PEM (private or public).
                (
                    alg,
                    DecodingKey::from_rsa_pem(&pem).context("parsing RSA PEM for introspection")?,
                )
            }
            "ES256" | "ES384" => {
                let path = cfg
                    .signing_key_path
                    .as_deref()
                    .context("ES* introspection requires signing_key_path")?;
                let pem = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("reading EC key from {path}"))?;
                let alg = if cfg.signing_algorithm == "ES384" {
                    Algorithm::ES384
                } else {
                    Algorithm::ES256
                };
                (
                    alg,
                    DecodingKey::from_ec_pem(&pem).context("parsing EC PEM for introspection")?,
                )
            }
            other => anyhow::bail!("unsupported signing algorithm for introspection: {other}"),
        };

        let mut validation = Validation::new(alg);
        // Pin the issuer to the gateway so a token minted by a DIFFERENT issuer
        // (even with a colliding secret) is not reported active.
        validation.set_issuer(&[cfg.issuer.as_str()]);
        // The gateway-minted tokens carry `aud` only when a resource was
        // requested; do not force audience validation here.
        validation.validate_aud = false;

        Ok(Self {
            decoding_key,
            validation,
        })
    }

    /// Introspect a token. Returns the RFC 7662 response object: an active token
    /// yields its claims (`active:true`, `scope`, `aud`, `exp`, `iat`, `sub`,
    /// `client_id`); anything unverifiable yields `{"active": false}`.
    pub fn introspect(&self, token: &str) -> Value {
        match decode::<Map<String, Value>>(token, &self.decoding_key, &self.validation) {
            Ok(data) => active_response(data.claims),
            // Any verification/expiry/issuer failure → inactive. Never leak why.
            Err(_) => json!({ "active": false }),
        }
    }
}

/// Delegate introspection of an opaque token to Ory Hydra's admin
/// introspection endpoint (RFC 7662). Used only when local verification reports
/// the token inactive AND `introspection_delegate` is configured — Hydra owns
/// introspection for the tokens it issued.
///
/// Returns Hydra's introspection JSON, or `{"active": false}` on any transport
/// error (fail-closed: an introspection outage never reports a token active).
pub async fn delegate_to_hydra(
    http_client: &reqwest::Client,
    hydra_admin_url: &str,
    token: &str,
) -> Value {
    let url = format!(
        "{}/admin/oauth2/introspect",
        hydra_admin_url.trim_end_matches('/')
    );
    let resp = http_client
        .post(&url)
        .form(&[("token", token)])
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            // Size-capped read (64 KiB). Any error — over-cap, transport, or
            // malformed JSON — is treated as INACTIVE (fail-closed: a token we
            // cannot positively confirm active is denied).
            crate::auth::http_body::read_capped_json(
                r,
                crate::auth::http_body::MAX_UPSTREAM_BODY_BYTES,
            )
            .await
            .unwrap_or_else(|_| json!({ "active": false }))
        }
        _ => json!({ "active": false }),
    }
}

/// Build the RFC 7662 active-token response from verified claims. Only surfaces
/// the standard introspection fields that are present.
fn active_response(claims: Map<String, Value>) -> Value {
    let mut out = Map::new();
    out.insert("active".to_string(), json!(true));
    for field in ["scope", "client_id", "sub", "aud", "exp", "iat", "iss", "jti"] {
        if let Some(v) = claims.get(field) {
            out.insert(field.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::identity::Identity;
    use crate::auth::jwt_mint::JwtMinter;

    fn hs256_cfg() -> JwtConfig {
        JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("introspection-test-secret-key".into()),
            signing_key_path: None,
            issuer: "flint-gate".into(),
            default_ttl_seconds: 300,
        }
    }

    #[tokio::test]
    async fn active_token_round_trips_with_claims() {
        let cfg = hs256_cfg();
        let minter = JwtMinter::from_config(&cfg).await.unwrap();
        let verifier = TokenVerifier::from_jwt_config(&cfg).await.unwrap();

        let identity = Identity {
            id: "svc-1".into(),
            ..Default::default()
        };
        let extra = json!({ "client_id": "svc-1", "scope": "svc.read" });
        let token = minter.mint(&identity, Some(&extra), Some(300)).unwrap();

        let resp = verifier.introspect(&token);
        assert_eq!(resp["active"], true);
        assert_eq!(resp["client_id"], "svc-1");
        assert_eq!(resp["scope"], "svc.read");
        assert_eq!(resp["sub"], "svc-1");
        assert!(resp.get("exp").is_some());
    }

    #[tokio::test]
    async fn garbage_token_is_inactive() {
        let verifier = TokenVerifier::from_jwt_config(&hs256_cfg()).await.unwrap();
        let resp = verifier.introspect("not.a.jwt");
        assert_eq!(resp["active"], false);
        // Inactive response leaks nothing else.
        assert_eq!(resp.as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn token_from_different_secret_is_inactive() {
        // A token minted with a DIFFERENT secret must not verify — no fail-open.
        let other = JwtConfig {
            signing_key_secret: Some("a-totally-different-secret-value".into()),
            ..hs256_cfg()
        };
        let foreign = JwtMinter::from_config(&other).await.unwrap();
        let token = foreign
            .mint(&Identity { id: "x".into(), ..Default::default() }, None, Some(300))
            .unwrap();

        let verifier = TokenVerifier::from_jwt_config(&hs256_cfg()).await.unwrap();
        assert_eq!(verifier.introspect(&token)["active"], false);
    }

    #[tokio::test]
    async fn token_from_different_issuer_is_inactive() {
        // Same secret, different issuer → the issuer pin rejects it.
        let other_issuer = JwtConfig {
            issuer: "evil-issuer".into(),
            ..hs256_cfg()
        };
        let foreign = JwtMinter::from_config(&other_issuer).await.unwrap();
        let token = foreign
            .mint(&Identity { id: "x".into(), ..Default::default() }, None, Some(300))
            .unwrap();

        let verifier = TokenVerifier::from_jwt_config(&hs256_cfg()).await.unwrap();
        assert_eq!(verifier.introspect(&token)["active"], false);
    }

    // ── Hydra delegate seam ───────────────────────────────────────────────

    #[tokio::test]
    async fn delegate_forwards_and_returns_hydra_response() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/admin/oauth2/introspect"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(json!({ "active": true, "client_id": "hydra-client" })),
            )
            .mount(&server)
            .await;

        let resp =
            delegate_to_hydra(&reqwest::Client::new(), &server.uri(), "opaque-token").await;
        assert_eq!(resp["active"], true);
        assert_eq!(resp["client_id"], "hydra-client");
    }

    #[tokio::test]
    async fn delegate_fails_closed_on_transport_error() {
        // Unreachable Hydra → active:false, never active.
        let resp =
            delegate_to_hydra(&reqwest::Client::new(), "http://127.0.0.1:1", "tok").await;
        assert_eq!(resp["active"], false);
    }

    #[tokio::test]
    async fn delegate_does_not_follow_redirects() {
        // A Hydra 3xx must NOT be followed (token-exfiltration guard): with a
        // no-redirect client the 302 surfaces as a non-2xx → active:false.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(302)
                    .insert_header("location", "https://attacker.example/steal"),
            )
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resp = delegate_to_hydra(&client, &server.uri(), "opaque-token").await;
        assert_eq!(resp["active"], false);
    }
}
