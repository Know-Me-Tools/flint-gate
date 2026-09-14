//! Expiry-aware Kratos session caching with a stale-refill fence.

use super::GateCache;
use crate::auth::identity::Identity;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::debug;

#[cfg(feature = "redis-l2")]
use redis::AsyncCommands;

#[derive(Serialize, Deserialize)]
struct CachedSession {
    generation: u64,
    identity: Identity,
}

impl GateCache {
    /// Capture the generation that an authentication or cache lookup starts under.
    pub fn session_generation(&self) -> u64 {
        self.session_generation.load(Ordering::SeqCst)
    }

    /// Look up a session only if it belongs to the caller's captured generation.
    pub async fn get_session_if_current(
        &self,
        credential: &str,
        generation: u64,
    ) -> Option<Identity> {
        if self.authority_readiness.is_some() {
            return None;
        }
        if self.session_generation() != generation {
            return None;
        }
        let key = Self::session_key(credential);
        if let Some(value) = self.sessions.get(&key).await {
            match serde_json::from_value::<CachedSession>(value) {
                Ok(cached) if self.cached_session_is_current(&cached, generation) => {
                    debug!("session L1 cache hit");
                    return Some(cached.identity);
                }
                Ok(_) => self.remove_session_key(&key).await,
                Err(error) => {
                    debug!(%error, "failed to deserialize L1 cached session");
                    self.remove_session_key(&key).await;
                }
            }
        }

        #[cfg(feature = "redis-l2")]
        if let Some(ref connection) = self.l2 {
            let redis_key = Self::l2_session_key(&key);
            let mut connection = connection.clone();
            match connection.get::<_, Option<String>>(&redis_key).await {
                Ok(Some(json)) => match serde_json::from_str::<CachedSession>(&json) {
                    Ok(cached) if self.cached_session_is_current(&cached, generation) => {
                        debug!("session L2 cache hit");
                        if let Ok(value) = serde_json::to_value(&cached) {
                            self.sessions.insert(key, value).await;
                        }
                        if self.session_generation() == generation {
                            return Some(cached.identity);
                        }
                    }
                    Ok(_) => self.remove_session_key(&key).await,
                    Err(error) => {
                        debug!(%error, "failed to deserialize L2 cached session");
                        self.remove_session_key(&key).await;
                    }
                },
                Ok(None) => debug!("session L2 cache miss"),
                Err(error) => tracing::warn!(%error, "L2 Redis GET failed"),
            }
        }
        None
    }

    /// Look up a session under the generation current at method entry.
    pub async fn get_session(&self, credential: &str) -> Option<Identity> {
        let generation = self.session_generation();
        self.get_session_if_current(credential, generation).await
    }

    /// Publish an authenticated session only if invalidation has not advanced.
    pub async fn put_session_if_current(
        &self,
        credential: &str,
        identity: &Identity,
        generation: u64,
    ) -> bool {
        if self.authority_readiness.is_some() {
            return false;
        }
        let Some(ttl) = self.session_ttl(identity) else {
            return false;
        };
        #[cfg(not(feature = "redis-l2"))]
        let _ = ttl;
        if self.session_generation() != generation {
            return false;
        }
        let key = Self::session_key(credential);
        let cached = CachedSession {
            generation,
            identity: identity.clone(),
        };
        let Ok(value) = serde_json::to_value(&cached) else {
            return false;
        };

        #[cfg(feature = "redis-l2")]
        if let Some(ref connection) = self.l2 {
            let redis_key = Self::l2_session_key(&key);
            let Ok(json) = serde_json::to_string(&cached) else {
                return false;
            };
            let mut connection = connection.clone();
            if let Err(error) = connection
                .set_ex::<_, _, ()>(&redis_key, json, ttl.as_secs().max(1))
                .await
            {
                tracing::warn!(%error, "L2 Redis SET failed");
            }
            if self.session_generation() != generation {
                let _: Result<(), _> = connection.del(&redis_key).await;
                return false;
            }
        }

        if self.session_generation() != generation {
            return false;
        }
        self.sessions.insert(key.clone(), value).await;
        if self.session_generation() != generation {
            self.remove_session_key(&key).await;
            return false;
        }
        true
    }

    /// Store a session under the generation current at method entry.
    pub async fn put_session(&self, credential: &str, identity: &Identity) {
        let generation = self.session_generation();
        let _ = self
            .put_session_if_current(credential, identity, generation)
            .await;
    }

    /// Advance the global session fence before evicting one credential.
    pub async fn invalidate_session(&self, credential: &str) {
        self.session_generation.fetch_add(1, Ordering::SeqCst);
        let key = Self::session_key(credential);
        self.remove_session_key(&key).await;
    }

    fn cached_session_is_current(&self, cached: &CachedSession, generation: u64) -> bool {
        cached.generation == generation
            && self.session_generation() == generation
            && cached
                .identity
                .session_expires_at
                .is_none_or(|expiry| expiry > Utc::now())
    }

    pub(super) fn session_ttl(&self, identity: &Identity) -> Option<Duration> {
        let Some(expiry) = identity.session_expires_at else {
            return Some(self.session_ttl);
        };
        let remaining = (expiry - Utc::now()).to_std().ok()?;
        Some(remaining.min(self.session_ttl))
    }

    async fn remove_session_key(&self, key: &str) {
        self.sessions.invalidate(key).await;
        #[cfg(feature = "redis-l2")]
        if let Some(ref connection) = self.l2 {
            let redis_key = Self::l2_session_key(key);
            let mut connection = connection.clone();
            let _: Result<(), _> = connection.del(&redis_key).await;
        }
    }

    fn session_key(credential: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(credential.as_bytes());
        hex::encode(hasher.finalize())
    }

    #[cfg(feature = "redis-l2")]
    fn l2_session_key(hash: &str) -> String {
        format!("{}:session:{hash}", super::L2_PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::CacheConfig;
    use chrono::TimeDelta;

    #[tokio::test]
    async fn invalidation_blocks_a_late_refill() {
        let cache = GateCache::from_config(&CacheConfig::default());
        let generation = cache.session_generation();
        cache.invalidate_session("credential").await;

        let published = cache
            .put_session_if_current("credential", &Identity::anonymous("subject"), generation)
            .await;

        assert!(!published);
        assert!(cache.get_session("credential").await.is_none());
    }

    #[tokio::test]
    async fn an_expired_session_is_never_published() {
        let cache = GateCache::from_config(&CacheConfig::default());
        let identity = Identity {
            session_expires_at: Some(Utc::now() - TimeDelta::seconds(1)),
            ..Identity::anonymous("subject")
        };

        cache.put_session("credential", &identity).await;

        assert!(cache.get_session("credential").await.is_none());
    }

    #[tokio::test]
    async fn legacy_unstamped_cache_is_disabled_when_authority_is_configured() {
        let mut cache = GateCache::from_config(&CacheConfig::default());
        cache.set_authority_readiness(crate::authority::AuthorityReadiness::new());
        let generation = cache.session_generation();

        assert!(
            !cache
                .put_session_if_current("credential", &Identity::anonymous("subject"), generation,)
                .await
        );
        assert!(cache
            .get_session_if_current("credential", generation)
            .await
            .is_none());
    }
}
