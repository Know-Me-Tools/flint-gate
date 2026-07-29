/// Outbound JWT minting for upstream service authentication.
///
/// Mints signed JWTs with identity claims so that upstream services can trust
/// the forwarded identity without calling Kratos themselves.
use crate::auth::identity::Identity;
use crate::config::types::JwtConfig;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Default `kid` when an asymmetric key is configured without an explicit id.
/// Must match what the JWKS endpoint advertises for the same key.
pub const DEFAULT_KEY_ID: &str = "flint-gate-key";

/// Whether an algorithm is asymmetric, and so needs a `kid` for key selection.
/// HMAC verifiers share the secret and have no key to choose between.
fn is_asymmetric(alg: Algorithm) -> bool {
    !matches!(
        alg,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    )
}

/// A configured JWT minter. Created from [`JwtConfig`] or a DB-sourced key.
#[derive(Clone)]
pub struct JwtMinter {
    algorithm: Algorithm,
    encoding_key: EncodingKey,
    issuer: String,
    default_ttl_seconds: u64,
    /// `kid` header written on every minted token.
    ///
    /// Asymmetric verifiers select a JWKS key by `kid`; without one they cannot
    /// know which public key to try and reject the token outright (flint-forge
    /// returns `IdentityError::UnknownKid`). A token signed RS256 but carrying
    /// no `kid` therefore fails at every asymmetric upstream, which reads as a
    /// signature problem rather than a missing header.
    ///
    /// `None` for HMAC, where there is no key to select.
    key_id: Option<String>,
}

/// Thread-safe optional JWT minter — `None` when JWT minting is not configured.
pub type SharedJwtMinter = Arc<RwLock<Option<JwtMinter>>>;

impl JwtMinter {
    /// Build a [`JwtMinter`] from [`JwtConfig`].
    pub async fn from_config(cfg: &JwtConfig) -> Result<Self> {
        let (algorithm, encoding_key) = Self::load_encoding_key(cfg).await?;
        // Asymmetric algorithms need a `kid` so verifiers can pick the key.
        // Explicit config wins; otherwise fall back to a stable default that
        // matches what the JWKS endpoint advertises.
        let key_id = if is_asymmetric(algorithm) {
            Some(
                cfg.signing_key_id
                    .clone()
                    .unwrap_or_else(|| DEFAULT_KEY_ID.to_string()),
            )
        } else {
            None
        };
        Ok(Self {
            algorithm,
            encoding_key,
            issuer: cfg.issuer.clone(),
            default_ttl_seconds: cfg.default_ttl_seconds,
            key_id,
        })
    }

    /// Build a [`JwtMinter`] from a DB-sourced key, preferring it over config.
    /// Falls back to config when the DB key is unavailable or uses an unsupported format.
    pub async fn from_db_or_config(
        db: Option<&crate::db::Database>,
        cfg: &JwtConfig,
    ) -> Result<Self> {
        if let Some(db) = db {
            if let Ok(Some(key)) = db.get_active_signing_key().await {
                match Self::from_db_key(&key, &cfg.issuer, cfg.default_ttl_seconds) {
                    Ok(minter) => {
                        tracing::info!(
                            key_id = %key.id,
                            algorithm = %key.algorithm,
                            "loaded JWT signing key from database"
                        );
                        return Ok(minter);
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            key_id = %key.id,
                            "failed to load DB signing key — falling back to config"
                        );
                    }
                }
            }
        }
        Self::from_config(cfg).await
    }

    /// Build from a DB-sourced key row.
    fn from_db_key(
        key: &crate::db::JwtSigningKey,
        issuer: &str,
        default_ttl_seconds: u64,
    ) -> Result<Self> {
        let algorithm = match key.algorithm.as_str() {
            "HS256" => Algorithm::HS256,
            "HS384" => Algorithm::HS384,
            "HS512" => Algorithm::HS512,
            "RS256" => Algorithm::RS256,
            "RS384" => Algorithm::RS384,
            "RS512" => Algorithm::RS512,
            "ES256" => Algorithm::ES256,
            "ES384" => Algorithm::ES384,
            other => bail!("unsupported algorithm in DB key: {other}"),
        };

        let encoding_key = match algorithm {
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                EncodingKey::from_secret(key.private_key.as_bytes())
            }
            Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => {
                EncodingKey::from_rsa_pem(key.private_key.as_bytes())
                    .context("parsing DB-sourced RSA PEM")?
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                EncodingKey::from_ec_pem(key.private_key.as_bytes())
                    .context("parsing DB-sourced EC PEM")?
            }
            _ => bail!("unsupported algorithm: {algorithm:?}"),
        };

        Ok(Self {
            algorithm,
            encoding_key,
            issuer: issuer.to_string(),
            default_ttl_seconds,
            // The DB row's id IS the key id — it is what the JWKS endpoint
            // publishes, so a rotated key is selectable by verifiers.
            key_id: is_asymmetric(algorithm).then(|| key.id.to_string()),
        })
    }

    async fn load_encoding_key(cfg: &JwtConfig) -> Result<(Algorithm, EncodingKey)> {
        let (algorithm, encoding_key) = match cfg.signing_algorithm.as_str() {
            "HS256" | "HS384" | "HS512" => {
                let secret = cfg
                    .signing_key_secret
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .context("HS* algorithm requires signing_key_secret")?;
                let alg = match cfg.signing_algorithm.as_str() {
                    "HS384" => Algorithm::HS384,
                    "HS512" => Algorithm::HS512,
                    _ => Algorithm::HS256,
                };
                (alg, EncodingKey::from_secret(secret.as_bytes()))
            }
            "RS256" | "RS384" | "RS512" => {
                let path = cfg
                    .signing_key_path
                    .as_deref()
                    .context("RS* algorithm requires signing_key_path")?;
                let pem = tokio::fs::read(path)
                    .await
                    .with_context(|| format!("reading RSA key from {path}"))?;
                let alg = match cfg.signing_algorithm.as_str() {
                    "RS384" => Algorithm::RS384,
                    "RS512" => Algorithm::RS512,
                    _ => Algorithm::RS256,
                };
                (
                    alg,
                    EncodingKey::from_rsa_pem(&pem).context("parsing RSA PEM")?,
                )
            }
            "ES256" | "ES384" => {
                let path = cfg
                    .signing_key_path
                    .as_deref()
                    .context("ES* algorithm requires signing_key_path")?;
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
                    EncodingKey::from_ec_pem(&pem).context("parsing EC PEM")?,
                )
            }
            other => bail!("unsupported signing algorithm: {other}"),
        };
        Ok((algorithm, encoding_key))
    }

    /// Mint a JWT for the given identity, merging in `additional_claims`.
    pub fn mint(
        &self,
        identity: &Identity,
        additional_claims: Option<&Value>,
        ttl_seconds: Option<u64>,
    ) -> Result<String> {
        let now = Utc::now().timestamp();
        let ttl = ttl_seconds.unwrap_or(self.default_ttl_seconds) as i64;

        let mut claims = json!({
            "iss": self.issuer,
            "sub": identity.id,
            "iat": now,
            "exp": now + ttl,
            "jti": Uuid::new_v4().to_string(),
        });

        // Merge identity traits into claims
        if let Value::Object(traits) = &identity.traits {
            if let Value::Object(map) = &mut claims {
                for (k, v) in traits {
                    map.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }

        // Merge additional_claims (override existing keys)
        if let Some(Value::Object(extra)) = additional_claims {
            if let Value::Object(map) = &mut claims {
                for (k, v) in extra {
                    map.insert(k.clone(), v.clone());
                }
            }
        }

        let mut header = Header::new(self.algorithm);
        header.kid = self.key_id.clone();
        encode(&header, &claims, &self.encoding_key).context("encoding JWT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::JwtConfig;

    fn hs256_config() -> JwtConfig {
        JwtConfig {
            signing_algorithm: "HS256".to_string(),
            signing_key_secret: Some("test-secret-key-minimum-length".to_string()),
            signing_key_path: None,
            issuer: "test-issuer".to_string(),
            default_ttl_seconds: 300,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn mint_hs256_jwt() {
        let minter = JwtMinter::from_config(&hs256_config()).await.unwrap();
        let identity = Identity {
            id: "user-123".to_string(),
            ..Default::default()
        };
        let token = minter.mint(&identity, None, None).unwrap();
        assert!(!token.is_empty());
        // JWT has 3 parts separated by '.'
        assert_eq!(token.split('.').count(), 3);
    }

    #[tokio::test]
    async fn mint_with_additional_claims() {
        let minter = JwtMinter::from_config(&hs256_config()).await.unwrap();
        let identity = Identity {
            id: "user-456".to_string(),
            ..Default::default()
        };
        let extra = json!({"scope": "chat", "org": "acme"});
        let token = minter.mint(&identity, Some(&extra), Some(60)).unwrap();
        assert!(!token.is_empty());
    }
}
