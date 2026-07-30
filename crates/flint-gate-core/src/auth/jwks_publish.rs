//! JWKS publication — `GET /.well-known/jwks.json`.
//!
//! The gate mints outbound JWTs so upstream services can trust a forwarded
//! identity without calling Kratos themselves. Those services verify
//! asymmetrically: they read the token's `kid`, fetch a JWKS, and select the
//! matching public key.
//!
//! Without this endpoint that handshake cannot complete. A gate configured for
//! RS256 produces tokens no upstream can verify, because there is nowhere to
//! fetch the public half from — flint-forge, for instance, fails with
//! `IdentityError::UnknownKid` before it ever checks a signature.
//!
//! # What is served
//!
//! Only **public** key material, sourced from the `public_key` column that
//! already accompanies every signing key. The private half never leaves the
//! process, and this module has no access to it by construction — it reads the
//! `JwtSigningKeyPublic` projection, not the full row.
//!
//! # Rotation
//!
//! Every non-expired key is published, not just the active one. A verifier that
//! cached a token signed by the previous key must still be able to resolve it
//! during the overlap window; publishing only the active key would reject
//! in-flight tokens the moment a rotation lands.

use axum::{http::StatusCode, response::IntoResponse, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rsa::{
    pkcs1::DecodeRsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts, RsaPublicKey,
};
use serde::Serialize;
use serde_json::json;

/// Path this endpoint is mounted at. The RFC 7517 well-known location, so
/// verifiers configured with only an issuer URL can discover it.
pub const JWKS_PATH: &str = "/.well-known/jwks.json";

/// A single JSON Web Key, in the shape RFC 7517 defines.
#[derive(Debug, Clone, Serialize)]
pub struct Jwk {
    /// Key type — `RSA` or `EC`.
    pub kty: String,
    /// Intended use. Always `sig`: these keys verify signatures, never encrypt.
    #[serde(rename = "use")]
    pub use_: String,
    /// Algorithm this key signs with, e.g. `RS256`.
    pub alg: String,
    /// Key id. This is what a verifier matches the token's `kid` header against.
    pub kid: String,
    /// Base64url-encoded RSA modulus (RFC 7518 §6.3.1.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    /// Base64url-encoded RSA public exponent (RFC 7518 §6.3.1.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,
    /// PEM-encoded public key retained as a compatibility extension. RSA
    /// consumers use the standard `n`/`e` members above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pem: Option<String>,
}

/// The JWKS document.
#[derive(Debug, Clone, Serialize)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

fn rsa_parameters(public_key_pem: &str) -> anyhow::Result<(String, String)> {
    let key = RsaPublicKey::from_public_key_pem(public_key_pem)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(public_key_pem))
        .map_err(|e| anyhow::anyhow!("parsing RSA public key for JWKS: {e}"))?;
    Ok((
        URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
        URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
    ))
}

/// Whether an algorithm is asymmetric and therefore publishable.
///
/// HMAC keys are shared secrets. Publishing one would hand every reader the
/// ability to mint tokens, so they are excluded unconditionally — a gate
/// running HS256 correctly serves an empty key set.
fn is_publishable(algorithm: &str) -> bool {
    algorithm.starts_with("RS") || algorithm.starts_with("ES") || algorithm.starts_with("PS")
}

/// Build a JWKS from the gate's public signing keys.
///
/// HMAC keys are filtered out (see [`is_publishable`]), so a gate configured
/// for HS256 yields `{"keys":[]}` rather than leaking a shared secret.
#[must_use]
pub fn build_jwks(keys: &[crate::db::JwtSigningKeyPublic]) -> anyhow::Result<JwkSet> {
    let mut jwks = Vec::new();
    for key in keys.iter().filter(|k| is_publishable(&k.algorithm)) {
        let (n, e) = if key.algorithm.starts_with("RS") || key.algorithm.starts_with("PS") {
            let (n, e) = rsa_parameters(&key.public_key)?;
            (Some(n), Some(e))
        } else {
            (None, None)
        };
        jwks.push(Jwk {
            kty: if key.algorithm.starts_with("ES") {
                "EC".to_string()
            } else {
                "RSA".to_string()
            },
            use_: "sig".to_string(),
            alg: key.algorithm.clone(),
            kid: key.id.clone(),
            n,
            e,
            pem: Some(key.public_key.clone()),
        });
    }
    Ok(JwkSet { keys: jwks })
}

/// Axum handler for `GET /.well-known/jwks.json`.
///
/// Returns an empty key set rather than an error when no asymmetric key is
/// configured: an empty JWKS is a valid document meaning "this issuer publishes
/// no verifiable keys", which is the honest answer for an HMAC deployment. A
/// 404 or 500 here would read as a misconfigured gate.
pub async fn jwks_handler(db: Option<std::sync::Arc<crate::db::Database>>) -> impl IntoResponse {
    let Some(db) = db else {
        return (StatusCode::OK, Json(json!({ "keys": [] }))).into_response();
    };

    match db.list_signing_keys().await {
        Ok(keys) => match build_jwks(&keys) {
            Ok(set) => (StatusCode::OK, Json(json!(set))).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "failed to encode signing keys as JWKS");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "signing keys unavailable" })),
                )
                    .into_response()
            }
        },
        Err(e) => {
            // Never surface the database error: it may name schemas or hosts.
            tracing::error!(error = %e, "failed to load signing keys for JWKS");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "signing keys unavailable" })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rsa::{pkcs8::EncodePublicKey, RsaPrivateKey};

    fn key(id: &str, algorithm: &str) -> crate::db::JwtSigningKeyPublic {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, 1024).expect("test RSA key");
        let public_key = private
            .to_public_key()
            .to_public_key_pem(Default::default())
            .expect("test public PEM");
        crate::db::JwtSigningKeyPublic {
            id: id.to_string(),
            algorithm: algorithm.to_string(),
            public_key,
            active: true,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn publishes_an_rsa_key_with_its_kid() {
        let set = build_jwks(&[key("gate-key-1", "RS256")]).expect("valid JWKS");
        assert_eq!(set.keys.len(), 1);
        assert_eq!(set.keys[0].kid, "gate-key-1");
        assert_eq!(set.keys[0].kty, "RSA");
        assert!(set.keys[0].n.as_deref().is_some_and(|n| !n.is_empty()));
        assert!(set.keys[0].e.as_deref().is_some_and(|e| !e.is_empty()));
        assert_eq!(set.keys[0].use_, "sig");
    }

    #[test]
    fn published_rsa_key_is_consumable_by_jsonwebtoken() {
        let set = build_jwks(&[key("gate-key-1", "RS256")]).expect("valid JWKS");
        let value = serde_json::to_value(set).expect("JWKS JSON");
        let standard: jsonwebtoken::jwk::JwkSet =
            serde_json::from_value(value).expect("RFC 7517 shape");
        let selected = standard.find("gate-key-1").expect("kid is selectable");
        jsonwebtoken::DecodingKey::from_jwk(selected).expect("key is usable for verification");
    }

    #[test]
    fn never_publishes_an_hmac_secret() {
        // The whole point of the filter: an HS256 "public_key" is the shared
        // secret. Publishing it would let any reader mint valid tokens.
        let set = build_jwks(&[key("hmac", "HS256")]).expect("valid JWKS");
        assert!(set.keys.is_empty(), "HMAC keys must never be published");
    }

    #[test]
    fn publishes_every_asymmetric_key_for_rotation_overlap() {
        // A verifier holding a token signed by the outgoing key must still be
        // able to resolve it, so both are served during rotation.
        let set = build_jwks(&[key("old", "RS256"), key("new", "RS256")]).expect("valid JWKS");
        assert_eq!(set.keys.len(), 2);
    }

    #[test]
    fn marks_ec_keys_with_the_ec_key_type() {
        let set = build_jwks(&[key("ec", "ES256")]).expect("valid JWKS");
        assert_eq!(set.keys[0].kty, "EC");
    }

    #[test]
    fn an_empty_key_set_serializes_as_a_valid_document() {
        // "no verifiable keys" is a legitimate answer, not an error.
        let json =
            serde_json::to_string(&build_jwks(&[]).expect("valid JWKS")).expect("serializes");
        assert_eq!(json, r#"{"keys":[]}"#);
    }
}
