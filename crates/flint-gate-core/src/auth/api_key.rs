/// API key authenticator — extracts a key from a request header, hashes it
/// with SHA-256, and validates against the `api_keys` database table.
///
/// Valid keys are cached in an internal moka cache (5-minute TTL) so that
/// hot-path requests avoid a database round-trip.
use crate::auth::identity::Identity;
use crate::auth::{AuthError, AuthMethod, AuthResult, Authenticator};
use crate::config::types::ApiKeyAuthConfig;
use crate::db::Database;
use async_trait::async_trait;
use http::request::Parts;
use moka::future::Cache;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

/// Cached payload for a validated API key (everything except the raw key).
#[derive(Clone)]
struct CachedKey {
    client_id: String,
    scopes: Vec<String>,
}

/// API key authenticator.
pub struct ApiKeyAuthenticator {
    config: ApiKeyAuthConfig,
    db: Arc<Database>,
    /// Internal cache: SHA-256(key) → CachedKey.
    cache: Cache<String, CachedKey>,
}

impl ApiKeyAuthenticator {
    pub fn new(config: ApiKeyAuthConfig, db: Arc<Database>) -> Self {
        let cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(300))
            .build();
        Self { config, db, cache }
    }

    /// SHA-256 hash a raw API key and return the hex digest.
    pub fn hash_key(raw: &str) -> String {
        let mut h = Sha256::new();
        h.update(raw.as_bytes());
        hex::encode(h.finalize())
    }
}

#[async_trait]
impl Authenticator for ApiKeyAuthenticator {
    async fn authenticate(&self, parts: &Parts) -> Result<AuthResult, AuthError> {
        // ── Extract raw key from configured header ─────────────────────────
        let raw_key = parts
            .headers
            .get(self.config.header.as_str())
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AuthError::Unauthorized(format!("missing {} header", self.config.header))
            })?;

        let key_hash = Self::hash_key(raw_key);

        // ── Cache hit ──────────────────────────────────────────────────────
        if let Some(cached) = self.cache.get(&key_hash).await {
            debug!(client_id = %cached.client_id, "API key cache hit");
            return Ok(build_result(cached.client_id, cached.scopes));
        }

        // ── Database lookup ────────────────────────────────────────────────
        match self.db.validate_api_key(&key_hash).await {
            Ok(Some(record)) => {
                self.cache
                    .insert(
                        key_hash,
                        CachedKey {
                            client_id: record.client_id.clone(),
                            scopes: record.scopes.clone(),
                        },
                    )
                    .await;
                Ok(build_result(record.client_id, record.scopes))
            }
            Ok(None) => Err(AuthError::Unauthorized(
                "invalid or expired API key".to_string(),
            )),
            Err(e) => Err(AuthError::ProviderError(format!(
                "API key lookup failed: {e}"
            ))),
        }
    }
}

fn build_result(client_id: String, scopes: Vec<String>) -> AuthResult {
    AuthResult {
        identity: Identity {
            id: client_id.clone(),
            // An API key is a non-human service credential → authorize as a
            // Service principal (so Service:: policies apply and the client is
            // covered by the NHI revocation list).
            kind: crate::auth::identity::IdentityKind::Service,
            ..Default::default()
        },
        method: AuthMethod::ApiKey { client_id, scopes },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let h1 = ApiKeyAuthenticator::hash_key("my-secret-key");
        let h2 = ApiKeyAuthenticator::hash_key("my-secret-key");
        assert_eq!(h1, h2);
    }

    #[test]
    fn api_key_identity_is_a_service_principal() {
        // An API-key credential is a non-human service → Service kind, so
        // Service:: policies apply and it's covered by NHI revocation.
        let result = build_result("svc-client".into(), vec!["read".into()]);
        assert_eq!(
            result.identity.kind,
            crate::auth::identity::IdentityKind::Service
        );
        assert_eq!(
            crate::auth::identity::principal_kind_for(&result.identity),
            crate::authz::PrincipalKind::Service
        );
    }

    #[test]
    fn different_keys_produce_different_hashes() {
        let h1 = ApiKeyAuthenticator::hash_key("key-a");
        let h2 = ApiKeyAuthenticator::hash_key("key-b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = ApiKeyAuthenticator::hash_key("anything");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
