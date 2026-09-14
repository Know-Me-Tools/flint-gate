//! Redis-backed monotonic authority fence and versioned session entries.

use super::GateCache;
use crate::{
    auth::identity::Identity,
    authority::{
        AuthorityError, AuthorityFenceStore, AuthorityHighWater, AuthorityReadiness,
        AuthorityStamp, SharedAuthorityFence,
    },
};
use async_trait::async_trait;
use chrono::Utc;
use futures::StreamExt;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use uuid::Uuid;

const AUTHORITY_REDIS_OPERATION_TIMEOUT: Duration = Duration::from_millis(200);

const BOOTSTRAP_SCRIPT: &str = r#"
local function normalize(value)
  local normalized = string.gsub(value, '^0+', '')
  if normalized == '' then return '0' end
  return normalized
end
local function compare_decimal(left, right)
  left = normalize(left)
  right = normalize(right)
  if string.len(left) < string.len(right) then return -1 end
  if string.len(left) > string.len(right) then return 1 end
  if left < right then return -1 end
  if left > right then return 1 end
  return 0
end
local current = redis.call('HMGET', KEYS[1], 'deployment', 'incarnation', 'revision', 'sequence')
local function exact(values, offset)
  return values[1] == ARGV[offset]
     and values[2] == ARGV[offset + 1]
     and values[3] == ARGV[offset + 2]
     and values[4] == ARGV[offset + 3]
end
if exact(current, 6) then
  return 0
end
if ARGV[1] == 'missing' then
  if current[1] then
    return -1
  end
elseif not exact(current, 2) then
  return -1
end
local candidate_namespace = ARGV[6] .. ':' .. ARGV[7]
if redis.call('SISMEMBER', KEYS[2], candidate_namespace) == 1 then
  return -2
end
if current[1] == ARGV[6] and current[2] == ARGV[7]
   and (compare_decimal(ARGV[8], current[3]) < 0
        or compare_decimal(ARGV[9], current[4]) < 0) then
  return -2
end
if current[1] and (current[1] ~= ARGV[6] or current[2] ~= ARGV[7]) then
  redis.call('SADD', KEYS[2], current[1] .. ':' .. current[2])
end
redis.call('HSET', KEYS[1],
  'deployment', ARGV[6], 'incarnation', ARGV[7],
  'revision', ARGV[8], 'sequence', ARGV[9])
redis.call('HSETNX', KEYS[1], 'config_epoch', '0')
redis.call('HSETNX', KEYS[1], 'config_revision', '0')
redis.call('PUBLISH', ARGV[10], ARGV[11])
return 1
"#;

const ADVANCE_SCRIPT: &str = r#"
local function normalize(value)
  local normalized = string.gsub(value, '^0+', '')
  if normalized == '' then return '0' end
  return normalized
end
local function compare_decimal(left, right)
  left = normalize(left)
  right = normalize(right)
  if string.len(left) < string.len(right) then return -1 end
  if string.len(left) > string.len(right) then return 1 end
  if left < right then return -1 end
  if left > right then return 1 end
  return 0
end
local current = redis.call('HMGET', KEYS[1], 'deployment', 'incarnation', 'revision', 'sequence')
local function exact(values, offset)
  return values[1] == ARGV[offset]
     and values[2] == ARGV[offset + 1]
     and values[3] == ARGV[offset + 2]
     and values[4] == ARGV[offset + 3]
end
if exact(current, 5) then
  return 0
end
if not exact(current, 1) then
  return -1
end
if ARGV[1] ~= ARGV[5] or ARGV[2] ~= ARGV[6]
   or compare_decimal(ARGV[7], ARGV[3]) < 0
   or compare_decimal(ARGV[8], ARGV[4]) < 0 then
  return -2
end
redis.call('HSET', KEYS[1],
  'deployment', ARGV[5], 'incarnation', ARGV[6],
  'revision', ARGV[7], 'sequence', ARGV[8])
redis.call('HSETNX', KEYS[1], 'config_epoch', '0')
redis.call('HSETNX', KEYS[1], 'config_revision', '0')
redis.call('PUBLISH', ARGV[9], ARGV[10])
return 1
"#;

const ADVANCE_CONFIG_EPOCH_SCRIPT: &str = r#"
local function normalize(value)
  local normalized = string.gsub(value, '^0+', '')
  if normalized == '' then return '0' end
  return normalized
end
local function compare_decimal(left, right)
  left = normalize(left)
  right = normalize(right)
  if string.len(left) < string.len(right) then return -1 end
  if string.len(left) > string.len(right) then return 1 end
  if left < right then return -1 end
  if left > right then return 1 end
  return 0
end
if redis.call('HEXISTS', KEYS[1], 'deployment') == 0 then
  return -1
end
local current_epoch = redis.call('HGET', KEYS[1], 'config_epoch') or '0'
if ARGV[1] ~= 'force' then
  local current_revision = redis.call('HGET', KEYS[1], 'config_revision') or '0'
  local relation = compare_decimal(ARGV[2], current_revision)
  if relation < 0 then
    return -2
  end
  if relation == 0 then
    return current_epoch
  end
end
local advanced = redis.call('HINCRBY', KEYS[1], 'config_epoch', 1)
if ARGV[1] ~= 'force' then
  redis.call('HSET', KEYS[1], 'config_revision', ARGV[2])
end
redis.call('PUBLISH', ARGV[3], ARGV[4])
return advanced
"#;

const MATCH_SESSION_EPOCH_SCRIPT: &str = r#"
local current = redis.call('HMGET', KEYS[1], 'deployment', 'incarnation', 'revision', 'sequence', 'config_epoch')
if current[1] == ARGV[1] and current[2] == ARGV[2]
   and current[3] == ARGV[3] and current[4] == ARGV[4]
   and current[5] == ARGV[5] then
  return 1
end
return 0
"#;

const PUBLISH_SESSION_SCRIPT: &str = r#"
local current = redis.call('HMGET', KEYS[1], 'deployment', 'incarnation', 'revision', 'sequence', 'config_epoch')
if current[1] ~= ARGV[1] or current[2] ~= ARGV[2]
   or current[3] ~= ARGV[3] or current[4] ~= ARGV[4]
   or current[5] ~= ARGV[5] then
  return 0
end
redis.call('PSETEX', KEYS[2], ARGV[6], ARGV[7])
return 1
"#;

const LOAD_SESSION_SCRIPT: &str = r#"
local current = redis.call('HMGET', KEYS[1], 'deployment', 'incarnation', 'revision', 'sequence', 'config_epoch')
if current[1] ~= ARGV[1] or current[2] ~= ARGV[2]
   or current[3] ~= ARGV[3] or current[4] ~= ARGV[4]
   or current[5] ~= ARGV[5] then
  return {0, ''}
end
local payload = redis.call('GET', KEYS[2])
if not payload then
  return {1, ''}
end
return {2, payload}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Authorization,
    Cookie,
    SessionToken,
}

impl CredentialKind {
    fn key_component(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Cookie => "cookie",
            Self::SessionToken => "session-token",
        }
    }
}

#[derive(Clone)]
pub struct AuthoritySessionCacheKey {
    provider_digest: String,
    issuer: String,
    issuer_digest: String,
    credential_kind: CredentialKind,
    credential_digest: String,
    tenant_id: Option<Uuid>,
}

impl AuthoritySessionCacheKey {
    pub fn identity(
        provider_id: &str,
        issuer: &str,
        credential_kind: CredentialKind,
        credential: impl AsRef<[u8]>,
    ) -> Result<Self, AuthorityCacheError> {
        Self::new(provider_id, issuer, credential_kind, credential, None)
    }

    pub fn tenant(
        provider_id: &str,
        issuer: &str,
        credential_kind: CredentialKind,
        credential: impl AsRef<[u8]>,
        tenant_id: Uuid,
    ) -> Result<Self, AuthorityCacheError> {
        Self::new(
            provider_id,
            issuer,
            credential_kind,
            credential,
            Some(tenant_id),
        )
    }

    fn new(
        provider_id: &str,
        issuer: &str,
        credential_kind: CredentialKind,
        credential: impl AsRef<[u8]>,
        tenant_id: Option<Uuid>,
    ) -> Result<Self, AuthorityCacheError> {
        if provider_id.trim().is_empty() {
            return Err(AuthorityCacheError::InvalidKey("provider is blank"));
        }
        if issuer.trim().is_empty() {
            return Err(AuthorityCacheError::InvalidKey("issuer is blank"));
        }
        let credential = credential.as_ref();
        if credential.is_empty() {
            return Err(AuthorityCacheError::InvalidKey("credential is blank"));
        }
        Ok(Self {
            provider_digest: digest(&[b"provider", provider_id.as_bytes()]),
            issuer: issuer.to_owned(),
            issuer_digest: digest(&[b"issuer", issuer.as_bytes()]),
            credential_kind,
            credential_digest: digest(&[
                b"credential",
                credential_kind.key_component().as_bytes(),
                credential,
            ]),
            tenant_id,
        })
    }
}

#[derive(Debug, Error)]
pub enum AuthorityCacheError {
    #[error("authority cache key is invalid: {0}")]
    InvalidKey(&'static str),
    #[error("identity has no verified Kratos session ID")]
    MissingVerifiedSession,
    #[error("shared authority fence is unavailable")]
    SharedFenceUnavailable,
    #[error("authority cache serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("Redis authority cache operation failed")]
    Redis(#[from] redis::RedisError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedSessionEntry {
    schema_version: u8,
    config_epoch: u64,
    authority: AuthorityHighWater,
    kratos_issuer: String,
    verified_session_id: String,
    tenant_id: Option<Uuid>,
    identity: Identity,
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalSessionEntry {
    generation: u64,
    shared: SharedSessionEntry,
}

enum SessionLoad {
    FenceMismatch,
    Miss,
    Hit(Box<SharedSessionEntry>),
}

#[derive(Clone)]
pub struct RedisAuthorityFence {
    client: redis::Client,
    connection: ConnectionManager,
    source_digest: String,
    invalidation_channel: String,
}

impl RedisAuthorityFence {
    pub fn new(
        client: redis::Client,
        connection: ConnectionManager,
        source_key: &str,
    ) -> Result<Self, AuthorityCacheError> {
        if source_key.trim().is_empty() {
            return Err(AuthorityCacheError::InvalidKey("source key is blank"));
        }
        let source_digest = digest(&[b"source", source_key.as_bytes()]);
        Ok(Self {
            client,
            connection,
            invalidation_channel: format!("flint:v2:authority:{source_digest}:invalidate"),
            source_digest,
        })
    }

    fn fence_key(&self) -> String {
        fence_key_for(&self.source_digest)
    }

    fn superseded_key(&self) -> String {
        superseded_key_for(&self.source_digest)
    }

    fn session_key(&self, key: &AuthoritySessionCacheKey, water: &AuthorityHighWater) -> String {
        session_key_for(&self.source_digest, key, water)
    }

    async fn read_fence(&self) -> Result<SharedAuthorityFence, AuthorityCacheError> {
        let mut connection = self.connection.clone();
        let values: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            redis::cmd("HMGET")
                .arg(self.fence_key())
                .arg(&["deployment", "incarnation", "revision", "sequence"])
                .query_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        match values {
            (None, None, None, None) => Ok(SharedAuthorityFence::Missing),
            (Some(deployment), Some(incarnation), Some(revision), Some(sequence)) => {
                let deployment_id: Uuid = deployment
                    .parse()
                    .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)?;
                let authority_incarnation: Uuid = incarnation
                    .parse()
                    .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)?;
                if deployment_id.to_string() != deployment
                    || authority_incarnation.to_string() != incarnation
                {
                    return Err(AuthorityCacheError::SharedFenceUnavailable);
                }
                let revision = parse_canonical_i64(&revision, false)?;
                let outbox_sequence = parse_canonical_i64(&sequence, true)?;
                Ok(SharedAuthorityFence::Present(AuthorityHighWater {
                    stamp: AuthorityStamp {
                        deployment_id,
                        incarnation: authority_incarnation,
                        revision,
                    },
                    outbox_sequence,
                }))
            }
            _ => Err(AuthorityCacheError::SharedFenceUnavailable),
        }
    }

    pub(super) async fn read_configuration_state(&self) -> Result<(u64, u64), AuthorityCacheError> {
        let mut connection = self.connection.clone();
        let values: (Option<String>, Option<String>) = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            redis::cmd("HMGET")
                .arg(self.fence_key())
                .arg(&["config_epoch", "config_revision"])
                .query_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        let (epoch, revision) = match values {
            (Some(epoch), Some(revision)) => (epoch, revision),
            _ => return Err(AuthorityCacheError::SharedFenceUnavailable),
        };
        Ok((
            parse_canonical_u64(&epoch)?,
            parse_canonical_u64(&revision)?,
        ))
    }

    pub(super) async fn read_configuration_epoch(&self) -> Result<u64, AuthorityCacheError> {
        self.read_configuration_state()
            .await
            .map(|(epoch, _)| epoch)
    }

    pub(super) async fn advance_configuration_epoch(
        &self,
        config_revision: Option<u64>,
    ) -> Result<u64, AuthorityCacheError> {
        let (mode, revision, payload) = match config_revision {
            Some(revision) => (
                "revision",
                revision.to_string(),
                format!("config:{revision}"),
            ),
            None => ("force", "0".to_owned(), "config:force".to_owned()),
        };
        let mut connection = self.connection.clone();
        let result: i64 = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            redis::Script::new(ADVANCE_CONFIG_EPOCH_SCRIPT)
                .key(self.fence_key())
                .arg(mode)
                .arg(revision)
                .arg(&self.invalidation_channel)
                .arg(payload)
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        u64::try_from(result).map_err(|_| AuthorityCacheError::SharedFenceUnavailable)
    }

    async fn matches_session_epoch(
        &self,
        water: &AuthorityHighWater,
        config_epoch: u64,
    ) -> Result<bool, AuthorityCacheError> {
        let mut connection = self.connection.clone();
        let result: i64 = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            redis::Script::new(MATCH_SESSION_EPOCH_SCRIPT)
                .key(self.fence_key())
                .arg(water.stamp.deployment_id.to_string())
                .arg(water.stamp.incarnation.to_string())
                .arg(water.stamp.revision.to_string())
                .arg(water.outbox_sequence.to_string())
                .arg(config_epoch.to_string())
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        Ok(result == 1)
    }

    async fn bootstrap_inner(
        &self,
        observed: &SharedAuthorityFence,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        let (presence, observed_water) = match observed {
            SharedAuthorityFence::Missing => ("missing", None),
            SharedAuthorityFence::Present(water) => ("present", Some(water)),
        };
        let missing = AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: Uuid::nil(),
                incarnation: Uuid::nil(),
                revision: 1,
            },
            outbox_sequence: 0,
        };
        let observed_water = observed_water.unwrap_or(&missing);
        let mut connection = self.connection.clone();
        let script = redis::Script::new(BOOTSTRAP_SCRIPT);
        let result: i64 = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            script
                .key(self.fence_key())
                .key(self.superseded_key())
                .arg(presence)
                .arg(observed_water.stamp.deployment_id.to_string())
                .arg(observed_water.stamp.incarnation.to_string())
                .arg(observed_water.stamp.revision.to_string())
                .arg(observed_water.outbox_sequence.to_string())
                .arg(candidate.stamp.deployment_id.to_string())
                .arg(candidate.stamp.incarnation.to_string())
                .arg(candidate.stamp.revision.to_string())
                .arg(candidate.outbox_sequence.to_string())
                .arg(&self.invalidation_channel)
                .arg(fence_payload(candidate))
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| {
            warn!("Redis authority fence bootstrap timed out");
            AuthorityError::SharedFenceUnavailable
        })?
        .map_err(|error| {
            warn!(%error, "Redis authority fence bootstrap failed");
            AuthorityError::SharedFenceUnavailable
        })?;
        match result {
            0 | 1 => Ok(()),
            -1 => Err(AuthorityError::SharedFenceMismatch),
            -2 => Err(AuthorityError::SharedFenceRegression),
            _ => Err(AuthorityError::SharedFenceUnavailable),
        }
    }

    async fn advance_inner(
        &self,
        expected: &AuthorityHighWater,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        let mut connection = self.connection.clone();
        let script = redis::Script::new(ADVANCE_SCRIPT);
        let result: i64 = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            script
                .key(self.fence_key())
                .arg(expected.stamp.deployment_id.to_string())
                .arg(expected.stamp.incarnation.to_string())
                .arg(expected.stamp.revision.to_string())
                .arg(expected.outbox_sequence.to_string())
                .arg(candidate.stamp.deployment_id.to_string())
                .arg(candidate.stamp.incarnation.to_string())
                .arg(candidate.stamp.revision.to_string())
                .arg(candidate.outbox_sequence.to_string())
                .arg(&self.invalidation_channel)
                .arg(fence_payload(candidate))
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| {
            warn!("Redis authority fence advance timed out");
            AuthorityError::SharedFenceUnavailable
        })?
        .map_err(|error| {
            warn!(%error, "Redis authority fence advance failed");
            AuthorityError::SharedFenceUnavailable
        })?;
        match result {
            0 | 1 => Ok(()),
            -1 => Err(AuthorityError::SharedFenceMismatch),
            -2 => Err(AuthorityError::SharedFenceRegression),
            _ => Err(AuthorityError::SharedFenceUnavailable),
        }
    }

    async fn publish_session(
        &self,
        key: &AuthoritySessionCacheKey,
        water: &AuthorityHighWater,
        entry: &SharedSessionEntry,
        config_epoch: u64,
        ttl: Duration,
    ) -> Result<bool, AuthorityCacheError> {
        let ttl_millis = ttl.as_millis().min(i64::MAX as u128) as i64;
        if ttl_millis == 0 {
            return Ok(false);
        }
        let payload = serde_json::to_string(entry)?;
        let mut connection = self.connection.clone();
        let script = redis::Script::new(PUBLISH_SESSION_SCRIPT);
        let result: i64 = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            script
                .key(self.fence_key())
                .key(self.session_key(key, water))
                .arg(water.stamp.deployment_id.to_string())
                .arg(water.stamp.incarnation.to_string())
                .arg(water.stamp.revision.to_string())
                .arg(water.outbox_sequence.to_string())
                .arg(config_epoch.to_string())
                .arg(ttl_millis)
                .arg(payload)
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        Ok(result == 1)
    }

    async fn load_session(
        &self,
        key: &AuthoritySessionCacheKey,
        water: &AuthorityHighWater,
        config_epoch: u64,
    ) -> Result<SessionLoad, AuthorityCacheError> {
        let mut connection = self.connection.clone();
        let script = redis::Script::new(LOAD_SESSION_SCRIPT);
        let (outcome, payload): (i64, String) = tokio::time::timeout(
            AUTHORITY_REDIS_OPERATION_TIMEOUT,
            script
                .key(self.fence_key())
                .key(self.session_key(key, water))
                .arg(water.stamp.deployment_id.to_string())
                .arg(water.stamp.incarnation.to_string())
                .arg(water.stamp.revision.to_string())
                .arg(water.outbox_sequence.to_string())
                .arg(config_epoch.to_string())
                .invoke_async(&mut connection),
        )
        .await
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)??;
        match outcome {
            0 => Ok(SessionLoad::FenceMismatch),
            1 => Ok(SessionLoad::Miss),
            2 => Ok(SessionLoad::Hit(Box::new(serde_json::from_str(&payload)?))),
            _ => Err(AuthorityCacheError::SharedFenceUnavailable),
        }
    }

    pub async fn run_invalidation_listener(
        self: Arc<Self>,
        cache: Arc<GateCache>,
        cancellation: CancellationToken,
    ) {
        loop {
            if cancellation.is_cancelled() {
                return;
            }
            let outcome = self
                .listen_once(Arc::clone(&cache), cancellation.clone())
                .await;
            if cancellation.is_cancelled() {
                return;
            }
            if let Err(error) = outcome {
                warn!(%error, "Redis authority invalidation listener disconnected");
            }
            tokio::select! {
                _ = cancellation.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }

    async fn listen_once(
        &self,
        cache: Arc<GateCache>,
        cancellation: CancellationToken,
    ) -> redis::RedisResult<()> {
        let mut pubsub = self.client.get_async_pubsub().await?;
        pubsub.subscribe(&self.invalidation_channel).await?;
        let mut messages = pubsub.on_message();
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                message = messages.next() => match message {
                    Some(_) => cache.invalidate_local_sessions().await,
                    None => return Ok(()),
                }
            }
        }
    }
}

#[async_trait]
impl AuthorityFenceStore for RedisAuthorityFence {
    async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError> {
        self.read_fence().await.map_err(|error| {
            warn!(%error, "Redis authority fence observation failed");
            AuthorityError::SharedFenceUnavailable
        })
    }

    async fn bootstrap(
        &self,
        observed: &SharedAuthorityFence,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        self.bootstrap_inner(observed, candidate).await
    }

    async fn advance(
        &self,
        expected: &AuthorityHighWater,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        self.advance_inner(expected, candidate).await
    }

    async fn matches(&self, candidate: &AuthorityHighWater) -> Result<bool, AuthorityError> {
        Ok(self.observe().await? == SharedAuthorityFence::Present(candidate.clone()))
    }
}

impl GateCache {
    async fn current_configuration_epoch(&self) -> Result<u64, AuthorityCacheError> {
        let local = self
            .authority_config_epoch
            .load(std::sync::atomic::Ordering::SeqCst);
        if local == super::CONFIG_EPOCH_INVALID {
            return Err(AuthorityCacheError::SharedFenceUnavailable);
        }
        if local != super::CONFIG_EPOCH_UNINITIALIZED {
            return Ok(local);
        }
        let fence = self
            .authority_fence
            .as_ref()
            .ok_or(AuthorityCacheError::SharedFenceUnavailable)?;
        let observed = fence.read_configuration_epoch().await?;
        match self.authority_config_epoch.compare_exchange(
            super::CONFIG_EPOCH_UNINITIALIZED,
            observed,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        ) {
            Ok(_) => Ok(observed),
            Err(current) if current == super::CONFIG_EPOCH_INVALID => {
                Err(AuthorityCacheError::SharedFenceUnavailable)
            }
            Err(current) => Ok(current),
        }
    }

    pub fn authority_cache_high_water(&self) -> Option<AuthorityHighWater> {
        self.authority_readiness
            .as_ref()
            .and_then(AuthorityReadiness::acquire_cache_high_water)
    }

    fn authority_still_permits(
        &self,
        expected: &AuthorityHighWater,
        issuer: &str,
        identity: &Identity,
    ) -> bool {
        self.authority_readiness.as_ref().is_some_and(|readiness| {
            identity.session_id.as_deref().is_some_and(|session_id| {
                readiness.still_permits_session(expected, issuer, session_id, Utc::now())
            })
        })
    }

    fn authority_expected_is_current(&self, expected: &AuthorityHighWater) -> bool {
        self.authority_readiness.as_ref().is_some_and(|readiness| {
            readiness.acquire_cache_high_water().as_ref() == Some(expected)
        })
    }

    pub async fn get_versioned_session_if_current(
        &self,
        key: &AuthoritySessionCacheKey,
        expected: &AuthorityHighWater,
        generation: u64,
    ) -> Result<Option<Identity>, AuthorityCacheError> {
        let Some(fence) = self.authority_fence.as_ref() else {
            self.disable_authority_cache();
            return Ok(None);
        };
        if self.session_generation() != generation || !self.authority_expected_is_current(expected)
        {
            return Ok(None);
        }
        let config_epoch = match self.current_configuration_epoch().await {
            Ok(config_epoch) if self.session_generation() == generation => config_epoch,
            Ok(_) => return Ok(None),
            Err(error) => {
                self.disable_authority_cache();
                return Err(error);
            }
        };
        let local_key = fence.session_key(key, expected);
        if let Some(value) = self.sessions.get(&local_key).await {
            match serde_json::from_value::<LocalSessionEntry>(value) {
                Ok(local)
                    if local.generation == generation
                        && entry_is_current(&local.shared, key, expected, config_epoch)
                        && self.session_generation() == generation =>
                {
                    match fence.matches_session_epoch(expected, config_epoch).await {
                        Ok(true)
                            if self.authority_still_permits(
                                expected,
                                &key.issuer,
                                &local.shared.identity,
                            ) =>
                        {
                            debug!("versioned session L1 cache hit");
                            return Ok(Some(local.shared.identity));
                        }
                        Ok(true) => return Ok(None),
                        Ok(false) | Err(_) => {
                            warn!("shared authority fence could not attest an L1 cache hit");
                            self.disable_authority_cache();
                            self.sessions.invalidate(&local_key).await;
                            return Ok(None);
                        }
                    }
                }
                _ => self.sessions.invalidate(&local_key).await,
            }
        }
        let shared = match fence.load_session(key, expected, config_epoch).await {
            Ok(SessionLoad::Hit(shared)) => shared,
            Ok(SessionLoad::Miss) => return Ok(None),
            Ok(SessionLoad::FenceMismatch) | Err(_) => {
                warn!("shared authority fence could not attest an L2 cache lookup");
                self.disable_authority_cache();
                return Ok(None);
            }
        };
        if !entry_is_current(&shared, key, expected, config_epoch)
            || self.session_generation() != generation
            || !self.authority_still_permits(expected, &key.issuer, &shared.identity)
        {
            return Ok(None);
        }
        let value = serde_json::to_value(LocalSessionEntry {
            generation,
            shared: (*shared).clone(),
        })?;
        self.sessions.insert(local_key.clone(), value).await;
        let fence_current = fence.matches_session_epoch(expected, config_epoch).await;
        if self.session_generation() != generation
            || !matches!(fence_current, Ok(true))
            || !self.authority_still_permits(expected, &key.issuer, &shared.identity)
        {
            if !matches!(fence_current, Ok(true)) {
                self.disable_authority_cache();
            }
            self.sessions.invalidate(&local_key).await;
            return Ok(None);
        }
        debug!("versioned session L2 cache hit");
        Ok(Some(shared.identity))
    }

    pub async fn put_versioned_session_if_current(
        &self,
        key: &AuthoritySessionCacheKey,
        identity: &Identity,
        expected: &AuthorityHighWater,
        generation: u64,
    ) -> Result<bool, AuthorityCacheError> {
        let Some(fence) = self.authority_fence.as_ref() else {
            self.disable_authority_cache();
            return Ok(false);
        };
        let verified_session_id = identity
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(AuthorityCacheError::MissingVerifiedSession)?;
        let Some(ttl) = self.session_ttl(identity) else {
            return Ok(false);
        };
        if self.session_generation() != generation
            || !self.authority_expected_is_current(expected)
            || !self.authority_still_permits(expected, &key.issuer, identity)
        {
            return Ok(false);
        }
        let config_epoch = match self.current_configuration_epoch().await {
            Ok(config_epoch) if self.session_generation() == generation => config_epoch,
            Ok(_) => return Ok(false),
            Err(error) => {
                self.disable_authority_cache();
                return Err(error);
            }
        };
        let shared = SharedSessionEntry {
            schema_version: 3,
            config_epoch,
            authority: expected.clone(),
            kratos_issuer: key.issuer.clone(),
            verified_session_id: verified_session_id.to_owned(),
            tenant_id: key.tenant_id,
            identity: identity.clone(),
        };
        match fence
            .publish_session(key, expected, &shared, config_epoch, ttl)
            .await
        {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                warn!("shared authority fence rejected a session cache publication");
                self.disable_authority_cache();
                return Ok(false);
            }
        }
        if self.session_generation() != generation
            || !self.authority_still_permits(expected, &key.issuer, identity)
        {
            return Ok(false);
        }
        let local_key = fence.session_key(key, expected);
        self.sessions
            .insert(
                local_key.clone(),
                serde_json::to_value(LocalSessionEntry { generation, shared })?,
            )
            .await;
        let fence_current = fence.matches_session_epoch(expected, config_epoch).await;
        if self.session_generation() != generation
            || !matches!(fence_current, Ok(true))
            || !self.authority_still_permits(expected, &key.issuer, identity)
        {
            if !matches!(fence_current, Ok(true)) {
                self.disable_authority_cache();
            }
            self.sessions.invalidate(&local_key).await;
            return Ok(false);
        }
        Ok(true)
    }
}

fn entry_is_current(
    entry: &SharedSessionEntry,
    key: &AuthoritySessionCacheKey,
    expected: &AuthorityHighWater,
    config_epoch: u64,
) -> bool {
    entry.schema_version == 3
        && entry.config_epoch == config_epoch
        && entry.authority == *expected
        && entry.kratos_issuer == key.issuer
        && entry.tenant_id == key.tenant_id
        && entry.identity.session_id.as_deref() == Some(entry.verified_session_id.as_str())
        && entry
            .identity
            .session_expires_at
            .is_none_or(|expiry| expiry > Utc::now())
}

fn parse_canonical_i64(value: &str, zero_allowed: bool) -> Result<i64, AuthorityCacheError> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)?;
    let minimum = if zero_allowed { 0 } else { 1 };
    if parsed < minimum || parsed.to_string() != value {
        return Err(AuthorityCacheError::SharedFenceUnavailable);
    }
    Ok(parsed)
}

fn parse_canonical_u64(value: &str) -> Result<u64, AuthorityCacheError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| AuthorityCacheError::SharedFenceUnavailable)?;
    if parsed.to_string() != value || parsed >= super::CONFIG_EPOCH_INVALID {
        return Err(AuthorityCacheError::SharedFenceUnavailable);
    }
    Ok(parsed)
}

fn fence_payload(water: &AuthorityHighWater) -> String {
    format!(
        "{}:{}:{}:{}",
        water.stamp.deployment_id,
        water.stamp.incarnation,
        water.stamp.revision,
        water.outbox_sequence
    )
}

fn fence_key_for(source_digest: &str) -> String {
    format!("flint:v2:authority:{{{source_digest}}}:fence")
}

fn superseded_key_for(source_digest: &str) -> String {
    format!("flint:v2:authority:{{{source_digest}}}:superseded")
}

fn session_key_for(
    source_digest: &str,
    key: &AuthoritySessionCacheKey,
    water: &AuthorityHighWater,
) -> String {
    let partition = key.tenant_id.map_or_else(
        || "identity".to_owned(),
        |tenant| format!("tenant-{}", digest(&[b"tenant", tenant.as_bytes()])),
    );
    format!(
        "flint:v2:authority:{{{source_digest}}}:session:{}:{}:{}:{}:{}:{}:{}",
        water.stamp.deployment_id,
        water.stamp.incarnation,
        partition,
        key.provider_digest,
        key.issuer_digest,
        key.credential_kind.key_component(),
        key.credential_digest,
    )
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{
            AuthorityConsumer, AuthoritySource, PostgresAuthorityCursorStore,
            PostgresAuthoritySource, HIGH_WATER_FRESHNESS,
        },
        authz::AuthzEngine,
        config::types::{CacheConfig, GateConfig, SiteConfig},
        db::Database,
        proxy::Router,
    };
    use chrono::TimeDelta;
    use sqlx::postgres::PgPoolOptions;

    fn water(
        deployment: u128,
        incarnation: u128,
        revision: i64,
        sequence: i64,
    ) -> AuthorityHighWater {
        AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: Uuid::from_u128(deployment),
                incarnation: Uuid::from_u128(incarnation),
                revision,
            },
            outbox_sequence: sequence,
        }
    }

    fn identity(subject: &str) -> Identity {
        Identity {
            session_id: Some("verified-session".to_owned()),
            session_expires_at: Some(Utc::now() + TimeDelta::minutes(5)),
            ..Identity::anonymous(subject)
        }
    }

    async fn cache(url: &str) -> GateCache {
        let mut config = CacheConfig::default();
        config.l2.enabled = true;
        config.l2.redis_url = Some(url.to_owned());
        let mut cache = GateCache::from_config(&config);
        cache.connect_l2(&config).await.expect("Redis connection");
        cache
    }

    #[test]
    fn key_contract_hides_credentials_and_separates_authority_namespaces() {
        let source = digest(&[b"source", b"aso"]);
        let tenant_a = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "secret-cookie",
            Uuid::from_u128(1),
        )
        .unwrap();
        let tenant_b = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "secret-cookie",
            Uuid::from_u128(2),
        )
        .unwrap();
        let other_provider = AuthoritySessionCacheKey::tenant(
            "other-kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "secret-cookie",
            Uuid::from_u128(1),
        )
        .unwrap();
        let other_issuer = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://other-identity.example/",
            CredentialKind::Cookie,
            "secret-cookie",
            Uuid::from_u128(1),
        )
        .unwrap();
        let high_water = water(1, 2, 3, 4);
        let key_a = session_key_for(&source, &tenant_a, &high_water);
        let key_b = session_key_for(&source, &tenant_b, &high_water);
        let provider_key = session_key_for(&source, &other_provider, &high_water);
        let issuer_key = session_key_for(&source, &other_issuer, &high_water);
        let deployment_key = session_key_for(&source, &tenant_a, &water(9, 2, 3, 4));
        let fence_key = fence_key_for(&source);
        assert_ne!(key_a, key_b);
        assert_ne!(key_a, provider_key);
        assert_ne!(key_a, issuer_key);
        assert_ne!(key_a, deployment_key);
        assert_eq!(
            key_a
                .split('{')
                .nth(1)
                .and_then(|value| value.split('}').next()),
            fence_key
                .split('{')
                .nth(1)
                .and_then(|value| value.split('}').next())
        );
        assert!(!key_a.contains("secret-cookie"));
        assert!(!key_a.contains("identity.example"));
    }

    #[test]
    fn canonical_integer_parser_rejects_ambiguous_or_invalid_values() {
        assert_eq!(parse_canonical_i64("0", true).unwrap(), 0);
        assert_eq!(parse_canonical_i64("1", false).unwrap(), 1);
        assert!(parse_canonical_i64("01", true).is_err());
        assert!(parse_canonical_i64("-1", true).is_err());
        assert!(parse_canonical_i64("0", false).is_err());
    }

    #[tokio::test]
    #[ignore = "requires disposable Redis supplied through RA06C02_REDIS_URL"]
    async fn atomic_fence_and_versioned_sessions() {
        let url = std::env::var("RA06C02_REDIS_URL").expect("RA06C02_REDIS_URL is required");
        let source = format!("aso-test-{}", Uuid::new_v4());
        let mut cache_a = cache(&url).await;
        let mut cache_b = cache(&url).await;
        let fence_a = Arc::new(
            RedisAuthorityFence::new(
                cache_a.l2_client.clone().unwrap(),
                cache_a.l2.clone().unwrap(),
                &source,
            )
            .unwrap(),
        );
        let fence_b = Arc::new(
            RedisAuthorityFence::new(
                cache_b.l2_client.clone().unwrap(),
                cache_b.l2.clone().unwrap(),
                &source,
            )
            .unwrap(),
        );
        cache_a.authority_fence = Some(fence_a.clone());
        cache_b.authority_fence = Some(fence_b.clone());

        let first = water(1, 2, 9_007_199_254_740_993, 9_007_199_254_740_995);
        let next = water(1, 2, 9_007_199_254_740_994, 9_007_199_254_740_999);
        fence_a
            .bootstrap(&SharedAuthorityFence::Missing, &first)
            .await
            .unwrap();
        println!("authority_fence_check: bootstrap_cas");

        let key_a = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "same-credential",
            Uuid::from_u128(10),
        )
        .unwrap();
        let key_b = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "same-credential",
            Uuid::from_u128(11),
        )
        .unwrap();
        let generation = cache_a.session_generation();
        assert!(!cache_a
            .put_versioned_session_if_current(
                &key_a,
                &identity("unbootstrapped"),
                &first,
                generation
            )
            .await
            .unwrap());
        assert!(cache_b
            .get_versioned_session_if_current(&key_a, &first, cache_b.session_generation())
            .await
            .unwrap()
            .is_none());
        let readiness_a = AuthorityReadiness::new();
        let readiness_b = AuthorityReadiness::new();
        readiness_a.install_test_snapshot(first.clone());
        readiness_b.install_test_snapshot(first.clone());
        cache_a.authority_readiness = Some(readiness_a.clone());
        cache_b.authority_readiness = Some(readiness_b.clone());
        assert!(cache_a
            .put_versioned_session_if_current(&key_a, &identity("tenant-a"), &first, generation)
            .await
            .unwrap());
        assert_eq!(
            cache_b
                .get_versioned_session_if_current(&key_a, &first, cache_b.session_generation(),)
                .await
                .unwrap()
                .unwrap()
                .id,
            "tenant-a"
        );

        fence_b.advance(&first, &next).await.unwrap();
        assert!(!cache_a
            .put_versioned_session_if_current(&key_a, &identity("stale"), &first, generation)
            .await
            .unwrap());
        assert!(cache_a
            .get_versioned_session_if_current(&key_a, &first, generation)
            .await
            .unwrap()
            .is_none());
        assert!(readiness_a.bootstrap_required());
        println!("authority_fence_check: mismatch_requests_bootstrap");
        cache_a.authority_readiness = None;
        assert!(!cache_a
            .put_versioned_session_if_current(
                &key_a,
                &identity("unbootstrapped"),
                &next,
                generation
            )
            .await
            .unwrap());
        readiness_a.install_test_snapshot(next.clone());
        readiness_b.install_test_snapshot(next.clone());
        cache_a.authority_readiness = Some(readiness_a.clone());
        println!("authority_fence_check: delayed_refill_rejected");

        let current_generation = cache_a.session_generation();
        assert!(cache_a
            .put_versioned_session_if_current(
                &key_a,
                &identity("tenant-a-current"),
                &next,
                current_generation,
            )
            .await
            .unwrap());
        assert!(cache_a
            .get_versioned_session_if_current(&key_b, &next, current_generation)
            .await
            .unwrap()
            .is_none());
        assert!(cache_a
            .put_versioned_session_if_current(
                &key_b,
                &identity("tenant-b"),
                &next,
                current_generation,
            )
            .await
            .unwrap());
        assert_eq!(
            cache_a
                .get_versioned_session_if_current(&key_a, &next, current_generation)
                .await
                .unwrap()
                .unwrap()
                .id,
            "tenant-a-current"
        );
        println!("authority_fence_check: tenant_keys_isolated");

        assert!(matches!(
            fence_a.advance(&next, &first).await,
            Err(AuthorityError::SharedFenceRegression)
        ));
        let observed = fence_a.observe().await.unwrap();
        assert!(matches!(
            fence_a.bootstrap(&observed, &first).await,
            Err(AuthorityError::SharedFenceRegression)
        ));
        assert!(fence_a.matches(&next).await.unwrap());
        println!("authority_fence_check: bigint_monotonicity");

        let replacement = water(1, 3, 1, 0);
        fence_a.bootstrap(&observed, &replacement).await.unwrap();
        assert!(!fence_b.matches(&next).await.unwrap());
        assert!(cache_b
            .get_versioned_session_if_current(&key_a, &next, cache_b.session_generation(),)
            .await
            .unwrap()
            .is_none());
        let replacement_observed = fence_a.observe().await.unwrap();
        assert!(matches!(
            fence_b.bootstrap(&replacement_observed, &next).await,
            Err(AuthorityError::SharedFenceRegression)
        ));
        assert_eq!(
            fence_a.observe().await.unwrap(),
            SharedAuthorityFence::Present(replacement.clone())
        );
        let replacement_generation = cache_a.session_generation();
        let stale_config_epoch = cache_a.current_configuration_epoch().await.unwrap();
        readiness_a.install_test_snapshot(replacement.clone());
        readiness_b.install_test_snapshot(replacement.clone());
        assert!(cache_a
            .put_versioned_session_if_current(
                &key_a,
                &identity("replacement-current"),
                &replacement,
                replacement_generation,
            )
            .await
            .unwrap());
        assert_eq!(
            cache_b
                .get_versioned_session_if_current(
                    &key_a,
                    &replacement,
                    cache_b.session_generation(),
                )
                .await
                .unwrap()
                .unwrap()
                .id,
            "replacement-current"
        );
        let stale_entry = SharedSessionEntry {
            schema_version: 3,
            config_epoch: stale_config_epoch,
            authority: replacement.clone(),
            kratos_issuer: key_a.issuer.clone(),
            verified_session_id: "verified-session".to_owned(),
            tenant_id: key_a.tenant_id,
            identity: identity("pre-invalidation-refill"),
        };
        cache_a.invalidate_all_at_revision(1).await;
        let first_config_epoch = cache_a.current_configuration_epoch().await.unwrap();
        assert_eq!(first_config_epoch, stale_config_epoch + 1);
        assert!(cache_b
            .get_versioned_session_if_current(&key_a, &replacement, cache_b.session_generation(),)
            .await
            .unwrap()
            .is_none());
        cache_b.invalidate_all_at_revision(2).await;
        let second_config_epoch = cache_b.current_configuration_epoch().await.unwrap();
        assert_eq!(second_config_epoch, first_config_epoch + 1);
        assert!(!cache_a.invalidate_all_at_revision(1).await);
        assert_eq!(
            fence_a.read_configuration_state().await.unwrap().0,
            second_config_epoch
        );
        let first_revision_entry = SharedSessionEntry {
            config_epoch: first_config_epoch,
            identity: identity("first-config-revision-refill"),
            ..stale_entry.clone()
        };
        assert!(!fence_a
            .publish_session(
                &key_a,
                &replacement,
                &stale_entry,
                stale_config_epoch,
                Duration::from_secs(60),
            )
            .await
            .unwrap());
        assert!(!fence_a
            .publish_session(
                &key_a,
                &replacement,
                &first_revision_entry,
                first_config_epoch,
                Duration::from_secs(60),
            )
            .await
            .unwrap());
        cache_a.invalidate_all().await;
        assert_eq!(
            cache_a.current_configuration_epoch().await.unwrap(),
            second_config_epoch + 1
        );
        let mut connection = fence_a.connection.clone();
        let authority_session_exists: bool = redis::cmd("EXISTS")
            .arg(fence_a.session_key(&key_a, &replacement))
            .query_async(&mut connection)
            .await
            .unwrap();
        assert!(!authority_session_exists);
        let _: () = redis::cmd("PSETEX")
            .arg(fence_a.session_key(&key_a, &replacement))
            .arg(60_000)
            .arg(serde_json::to_string(&stale_entry).unwrap())
            .query_async(&mut connection)
            .await
            .unwrap();
        assert!(cache_a
            .get_versioned_session_if_current(&key_a, &replacement, cache_a.session_generation(),)
            .await
            .unwrap()
            .is_none());
        assert!(fence_a.matches(&replacement).await.unwrap());
        println!("authority_fence_check: incarnation_replacement_invalidates");

        let _: usize = redis::cmd("DEL")
            .arg(fence_a.fence_key())
            .arg(fence_a.superseded_key())
            .arg(fence_a.session_key(&key_a, &first))
            .arg(fence_a.session_key(&key_a, &next))
            .arg(fence_a.session_key(&key_b, &next))
            .query_async(&mut connection)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis supplied by the RA06c task 2.1 fixture"]
    async fn two_gate_replicas_reject_stale_authority() {
        let redis_url = std::env::var("RA06C02_REDIS_URL").expect("RA06C02_REDIS_URL is required");
        let reader_url = std::env::var("RA06C02_ASO_READER_DATABASE_URL")
            .expect("RA06C02_ASO_READER_DATABASE_URL is required");
        let owner_url = std::env::var("RA06C02_ASO_OWNER_DATABASE_URL")
            .expect("RA06C02_ASO_OWNER_DATABASE_URL is required");
        let gate_url = std::env::var("RA06C02_GATE_DATABASE_URL")
            .expect("RA06C02_GATE_DATABASE_URL is required");
        let source_key = format!("aso-two-gate-{}", Uuid::new_v4());

        let source_a = Arc::new(
            PostgresAuthoritySource::connect(&reader_url, 2)
                .await
                .unwrap(),
        );
        let source_b = Arc::new(
            PostgresAuthoritySource::connect(&reader_url, 2)
                .await
                .unwrap(),
        );
        let owner = PgPoolOptions::new()
            .max_connections(1)
            .connect(&owner_url)
            .await
            .unwrap();
        let gate_db = Arc::new(Database::connect(&gate_url, 4).await.unwrap());
        gate_db.migrate().await.unwrap();
        let mut config_listener = sqlx::postgres::PgListener::connect(&gate_url)
            .await
            .unwrap();
        config_listener
            .listen("flintgate_config_changed")
            .await
            .unwrap();

        let mut cache_a = cache(&redis_url).await;
        let mut cache_b = cache(&redis_url).await;
        let fence_a = Arc::new(
            RedisAuthorityFence::new(
                cache_a.l2_client.clone().unwrap(),
                cache_a.l2.clone().unwrap(),
                &source_key,
            )
            .unwrap(),
        );
        let fence_b = Arc::new(
            RedisAuthorityFence::new(
                cache_b.l2_client.clone().unwrap(),
                cache_b.l2.clone().unwrap(),
                &source_key,
            )
            .unwrap(),
        );
        let readiness_a = AuthorityReadiness::new();
        let readiness_b = AuthorityReadiness::new();
        cache_a.authority_fence = Some(fence_a.clone());
        cache_a.authority_readiness = Some(readiness_a.clone());
        cache_b.authority_fence = Some(fence_b.clone());
        cache_b.authority_readiness = Some(readiness_b.clone());

        let consumer_a = AuthorityConsumer::new(
            source_key.clone(),
            source_a.clone(),
            Arc::new(PostgresAuthorityCursorStore::new(gate_db.pool())),
            fence_a.clone(),
            readiness_a.clone(),
        );
        let consumer_b = AuthorityConsumer::new(
            source_key.clone(),
            source_b.clone(),
            Arc::new(PostgresAuthorityCursorStore::new(gate_db.pool())),
            fence_b.clone(),
            readiness_b.clone(),
        );
        consumer_a.bootstrap().await.unwrap();
        consumer_b.bootstrap().await.unwrap();
        let first = readiness_a.snapshot().high_water.unwrap();
        assert_eq!(readiness_b.snapshot().high_water.as_ref(), Some(&first));
        assert!(fence_a.matches(&first).await.unwrap());

        let mut held_publication = gate_db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(crate::db::CONFIGURATION_PUBLICATION_LOCK)
            .execute(&mut *held_publication)
            .await
            .unwrap();
        let concurrent_policy_id = format!("ra06c-publication-lock-{}", Uuid::new_v4());
        let concurrent_database = Arc::clone(&gate_db);
        let concurrent_writer = tokio::spawn(async move {
            concurrent_database
                .upsert_policy(
                    &concurrent_policy_id,
                    "permit(principal, action, resource);",
                    None,
                    None,
                    true,
                    None,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !concurrent_writer.is_finished(),
            "configuration writer must wait while publication holds the shared lock"
        );
        held_publication.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), concurrent_writer)
            .await
            .expect("configuration writer resumes after publication releases the lock")
            .unwrap()
            .unwrap();
        let concurrent_policy_notification =
            tokio::time::timeout(Duration::from_secs(2), config_listener.recv())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            crate::cache::classify_notification(concurrent_policy_notification.payload()),
            crate::cache::NotifyAction::Policies
        );

        let route_id = format!("ra06c-config-revision-{}", Uuid::new_v4());
        gate_db
            .upsert_route(&route_id, &serde_json::json!({}), 0)
            .await
            .unwrap();
        let first_notification =
            tokio::time::timeout(Duration::from_secs(2), config_listener.recv())
                .await
                .unwrap()
                .unwrap();
        let first_config_revision =
            crate::cache::notification_revision(first_notification.payload())
                .expect("route upsert carries a durable revision");
        assert_eq!(
            crate::cache::classify_notification(first_notification.payload()),
            crate::cache::NotifyAction::Routes
        );
        cache_a
            .invalidate_all_at_revision(first_config_revision)
            .await;
        assert!(gate_db.delete_route(&route_id).await.unwrap());
        let second_notification =
            tokio::time::timeout(Duration::from_secs(2), config_listener.recv())
                .await
                .unwrap()
                .unwrap();
        let second_config_revision =
            crate::cache::notification_revision(second_notification.payload())
                .expect("route delete carries a durable revision");
        assert_eq!(second_config_revision, first_config_revision + 1);
        cache_b
            .invalidate_all_at_revision(second_config_revision)
            .await;
        assert!(
            !cache_a
                .invalidate_all_at_revision(first_config_revision)
                .await
        );
        let stale_publication = crate::cache::publish_prepared_configuration(
            &cache_a,
            None,
            None,
            &None,
            crate::cache::PreparedConfiguration {
                revision: first_config_revision,
                router: None,
                policies: None,
                policy_count: None,
            },
        )
        .await;
        assert_eq!(
            stale_publication,
            crate::cache::ConfigurationReconcile::Failed
        );
        assert_eq!(
            cache_a
                .configuration_revision
                .load(std::sync::atomic::Ordering::SeqCst),
            crate::cache::CONFIG_REVISION_UNINITIALIZED
        );
        assert_eq!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_a, None).await,
            crate::cache::ConfigurationReconcile::Advanced
        );
        assert_eq!(
            cache_a.current_configuration_epoch().await.unwrap(),
            cache_b.current_configuration_epoch().await.unwrap()
        );
        let epoch_before_missed_notification = cache_a.current_configuration_epoch().await.unwrap();
        gate_db
            .upsert_route(&route_id, &serde_json::json!({"enabled": true}), 1)
            .await
            .unwrap();
        assert_ne!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_a, None).await,
            crate::cache::ConfigurationReconcile::Failed
        );
        assert_eq!(
            cache_a.current_configuration_epoch().await.unwrap(),
            epoch_before_missed_notification + 1
        );
        assert_ne!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_b, None).await,
            crate::cache::ConfigurationReconcile::Failed
        );
        assert_eq!(
            cache_a.current_configuration_epoch().await.unwrap(),
            cache_b.current_configuration_epoch().await.unwrap()
        );
        let revision_before_restart: i64 =
            sqlx::query_scalar("SELECT revision FROM config_revision WHERE singleton = true")
                .fetch_one(&gate_db.pool())
                .await
                .unwrap();
        gate_db
            .upsert_route(&route_id, &serde_json::json!({"enabled": true}), 2)
            .await
            .unwrap();
        gate_db
            .upsert_route(&route_id, &serde_json::json!({"enabled": true}), 3)
            .await
            .unwrap();
        let revision_after_restart: i64 =
            sqlx::query_scalar("SELECT revision FROM config_revision WHERE singleton = true")
                .fetch_one(&gate_db.pool())
                .await
                .unwrap();
        assert_eq!(revision_after_restart, revision_before_restart + 2);
        let epoch_before_restart = cache_a.current_configuration_epoch().await.unwrap();
        let mut restarted_cache = cache(&redis_url).await;
        let restarted_fence = Arc::new(
            RedisAuthorityFence::new(
                restarted_cache.l2_client.clone().unwrap(),
                restarted_cache.l2.clone().unwrap(),
                &source_key,
            )
            .unwrap(),
        );
        let restarted_readiness = AuthorityReadiness::new();
        restarted_readiness.install_test_snapshot(first.clone());
        restarted_cache.authority_fence = Some(restarted_fence);
        restarted_cache.authority_readiness = Some(restarted_readiness);
        assert_ne!(
            crate::cache::reconcile_configuration_revision(
                &gate_db.pool(),
                &restarted_cache,
                None,
            )
            .await,
            crate::cache::ConfigurationReconcile::Failed
        );
        assert_eq!(
            restarted_cache.current_configuration_epoch().await.unwrap(),
            epoch_before_restart + 1
        );
        assert_ne!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_a, None).await,
            crate::cache::ConfigurationReconcile::Failed
        );
        assert_ne!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_b, None).await,
            crate::cache::ConfigurationReconcile::Failed
        );
        println!("two_gate_fence_check: independent_replicas_bootstrap");

        let applied_before_empty_restart = cache_a
            .configuration_revision
            .load(std::sync::atomic::Ordering::SeqCst);
        let mut empty_restart_connection = fence_a.connection.clone();
        let _: usize = redis::cmd("DEL")
            .arg(fence_a.fence_key())
            .query_async(&mut empty_restart_connection)
            .await
            .unwrap();
        consumer_a.bootstrap().await.unwrap();
        assert_eq!(fence_a.read_configuration_state().await.unwrap(), (0, 0));
        assert_eq!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_a, None).await,
            crate::cache::ConfigurationReconcile::Advanced
        );
        let (restored_epoch, restored_revision) = fence_a.read_configuration_state().await.unwrap();
        assert!(restored_epoch > 0);
        assert_eq!(restored_revision, applied_before_empty_restart);
        assert_eq!(
            cache_a
                .configuration_revision
                .load(std::sync::atomic::Ordering::SeqCst),
            applied_before_empty_restart
        );
        assert_eq!(
            crate::cache::reconcile_configuration_revision(&gate_db.pool(), &cache_b, None).await,
            crate::cache::ConfigurationReconcile::Advanced
        );
        assert_eq!(
            cache_b.current_configuration_epoch().await.unwrap(),
            restored_epoch
        );
        println!("two_gate_fence_check: empty_redis_restart_repairs_configuration_state");

        let key_a = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "same-credential",
            Uuid::from_u128(41),
        )
        .unwrap();
        let key_b = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "same-credential",
            Uuid::from_u128(42),
        )
        .unwrap();
        readiness_a.install_test_snapshot(first.clone());
        let generation_a = cache_a.session_generation();
        assert!(cache_a
            .put_versioned_session_if_current(&key_a, &identity("tenant-a"), &first, generation_a,)
            .await
            .unwrap());
        readiness_b.install_test_snapshot(first.clone());
        assert!(cache_b
            .get_versioned_session_if_current(&key_b, &first, cache_b.session_generation(),)
            .await
            .unwrap()
            .is_none());
        readiness_b.install_test_snapshot(first.clone());
        assert_eq!(
            cache_b
                .get_versioned_session_if_current(&key_a, &first, cache_b.session_generation(),)
                .await
                .unwrap()
                .unwrap()
                .id,
            "tenant-a"
        );
        println!("two_gate_fence_check: cross_tenant_keys_isolated");

        for _ in 0..2 {
            sqlx::query("UPDATE aso.capabilities SET description = description WHERE false")
                .execute(&owner)
                .await
                .unwrap();
        }
        let observed = source_b.high_water().await.unwrap();
        let events = source_b
            .events_after(first.outbox_sequence, observed.outbox_sequence, 256)
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(events.last().unwrap().sequence, observed.outbox_sequence);
        println!("two_gate_fence_check: postgres_events_ordered_for_replay");

        consumer_b.synchronize().await.unwrap();
        let advanced = readiness_b.snapshot().high_water.unwrap();
        assert_eq!(advanced, observed);
        assert!(fence_a.matches(&advanced).await.unwrap());
        println!("two_gate_fence_check: second_replica_advances_shared_fence");

        consumer_a.synchronize().await.unwrap();
        assert_eq!(readiness_a.snapshot().high_water.as_ref(), Some(&advanced));
        consumer_b.synchronize().await.unwrap();
        assert_eq!(readiness_b.snapshot().high_water.as_ref(), Some(&advanced));
        assert!(fence_b.matches(&advanced).await.unwrap());
        assert!(matches!(
            fence_a.advance(&advanced, &first).await,
            Err(AuthorityError::SharedFenceRegression)
        ));
        assert_eq!(
            fence_a.observe().await.unwrap(),
            SharedAuthorityFence::Present(advanced.clone())
        );
        println!("two_gate_fence_check: delayed_duplicate_and_lower_delivery_are_monotonic");

        assert_eq!(cache_a.session_generation(), generation_a);
        readiness_a.install_test_snapshot(first.clone());
        assert!(cache_a
            .get_versioned_session_if_current(&key_a, &first, generation_a)
            .await
            .unwrap()
            .is_none());
        assert!(readiness_a.bootstrap_required());
        assert_eq!(cache_a.session_generation(), generation_a);
        println!("two_gate_fence_check: lost_pubsub_cannot_restore_stale_l1");

        readiness_a.install_test_snapshot(first.clone());
        assert!(!cache_a
            .put_versioned_session_if_current(
                &key_a,
                &identity("delayed-stale-refill"),
                &first,
                generation_a,
            )
            .await
            .unwrap());
        let mut connection = fence_a.connection.clone();
        let preserved_payload: String = redis::cmd("GET")
            .arg(fence_a.session_key(&key_a, &first))
            .query_async(&mut connection)
            .await
            .unwrap();
        let preserved_entry: SharedSessionEntry = serde_json::from_str(&preserved_payload).unwrap();
        assert_eq!(preserved_entry.identity.id, "tenant-a");
        assert!(readiness_a.bootstrap_required());
        println!("two_gate_fence_check: delayed_refill_rejected_by_shared_fence");

        consumer_a.bootstrap().await.unwrap();
        assert_eq!(readiness_a.snapshot().high_water.as_ref(), Some(&advanced));
        assert!(readiness_a.cache_ready());
        assert!(fence_a.matches(&advanced).await.unwrap());
        readiness_a.install_test_snapshot(advanced.clone());
        assert!(cache_a
            .get_versioned_session_if_current(&key_a, &advanced, generation_a)
            .await
            .unwrap()
            .is_none());
        println!("two_gate_fence_check: missed_events_recover_only_after_bootstrap");

        let listener_source_key = format!("aso-listener-{}", Uuid::new_v4());
        let mut listener_cache = cache(&redis_url).await;
        let listener_fence = Arc::new(
            RedisAuthorityFence::new(
                listener_cache.l2_client.clone().unwrap(),
                listener_cache.l2.clone().unwrap(),
                &listener_source_key,
            )
            .unwrap(),
        );
        listener_fence
            .bootstrap(&SharedAuthorityFence::Missing, &advanced)
            .await
            .unwrap();
        let listener_readiness = AuthorityReadiness::new();
        listener_readiness.install_test_snapshot(advanced.clone());
        listener_cache.authority_fence = Some(listener_fence);
        listener_cache.authority_readiness = Some(listener_readiness);
        let listener_cache = Arc::new(listener_cache);
        let mut listener_config = GateConfig {
            sites: vec![SiteConfig {
                id: "listener-site".to_owned(),
                domains: Vec::new(),
                default_auth: None,
                default_upstream: Some("http://127.0.0.1:9".to_owned()),
            }],
            ..GateConfig::default()
        };
        listener_config.database.override_yaml = true;
        let listener_router = Arc::new(tokio::sync::RwLock::new(Router::from_config(
            &listener_config,
        )));
        let listener_shared_config = Arc::new(tokio::sync::RwLock::new(listener_config));
        let listener_authz = Arc::new(AuthzEngine::empty());
        let listener_application_name = format!("ra06c-listener-{}", Uuid::new_v4());
        let listener_separator = if gate_url.contains('?') { '&' } else { '?' };
        let listener_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&format!(
                "{gate_url}{listener_separator}application_name={listener_application_name}"
            ))
            .await
            .unwrap();
        crate::cache::start_cache_invalidation_listener(
            listener_pool,
            Arc::clone(&listener_cache),
            "flintgate_config_changed".to_owned(),
            Some((
                Arc::clone(&listener_router),
                listener_shared_config,
                Arc::clone(&gate_db),
            )),
            Some((Arc::clone(&listener_authz), Arc::clone(&gate_db))),
            None,
        )
        .await;
        let listener_generation = listener_cache.session_generation();
        let listener_key = AuthoritySessionCacheKey::identity(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "listener-credential",
        )
        .unwrap();
        assert!(listener_cache
            .put_versioned_session_if_current(
                &listener_key,
                &identity("listener-session"),
                &advanced,
                listener_generation,
            )
            .await
            .unwrap());
        gate_db
            .upsert_route(&route_id, &serde_json::json!({"enabled": true}), 2)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while listener_cache.session_generation() == listener_generation {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("production listener advances the cache generation");
        assert!(listener_cache
            .get_versioned_session_if_current(&listener_key, &advanced, listener_generation)
            .await
            .unwrap()
            .is_none());

        let listener_connections_terminated: bool = sqlx::query_scalar(
            "SELECT COALESCE(bool_or(pg_terminate_backend(pid)), false)
               FROM pg_stat_activity
              WHERE application_name = $1
                AND pid <> pg_backend_pid()",
        )
        .bind(&listener_application_name)
        .fetch_one(&gate_db.pool())
        .await
        .unwrap();
        assert!(listener_connections_terminated);

        let generation_before_lost_notification = listener_cache.session_generation();
        let lost_route_id = format!("ra06c-lost-route-{}", Uuid::new_v4());
        let lost_policy_id = format!("ra06c-lost-policy-{}", Uuid::new_v4());
        let lost_route = serde_json::json!({
            "id": lost_route_id,
            "site": "listener-site",
            "match": { "path": "/ra06c/lost/**", "methods": [] },
            "upstream": "http://127.0.0.1:9",
            "auth": null,
            "hooks": {},
            "stream": {},
            "priority": 11,
            "enabled": true
        });
        let mut lost_notification_tx = gate_db.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO gate_routes (id, config, priority, enabled, updated_at)
             VALUES ($1, $2, 11, true, NOW())
             ON CONFLICT (id) DO UPDATE SET
               config = EXCLUDED.config,
               priority = EXCLUDED.priority,
               enabled = EXCLUDED.enabled,
               updated_at = NOW()",
        )
        .bind(&lost_route_id)
        .bind(&lost_route)
        .execute(&mut *lost_notification_tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO authz_policies
                 (id, policy_text, schema_json, entities_json, enabled, updated_at)
             VALUES ($1, 'permit(principal, action, resource);', NULL, NULL, true, NOW())
             ON CONFLICT (id) DO UPDATE SET
               policy_text = EXCLUDED.policy_text,
               schema_json = NULL,
               entities_json = NULL,
               enabled = true,
               updated_at = NOW()",
        )
        .bind(&lost_policy_id)
        .execute(&mut *lost_notification_tx)
        .await
        .unwrap();
        let durable_revision_without_notification: i64 = sqlx::query_scalar(
            "UPDATE config_revision
                SET revision = revision + 1
              WHERE singleton = true
          RETURNING revision",
        )
        .fetch_one(&mut *lost_notification_tx)
        .await
        .unwrap();
        lost_notification_tx.commit().await.unwrap();
        let durable_revision_without_notification =
            u64::try_from(durable_revision_without_notification).unwrap();
        let convergence = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let revision_applied = listener_cache
                    .configuration_revision
                    .load(std::sync::atomic::Ordering::SeqCst)
                    == durable_revision_without_notification;
                let route_published = listener_router
                    .read()
                    .await
                    .route_ids()
                    .any(|id| id == lost_route_id);
                let policy_published =
                    listener_authz
                        .snapshot()
                        .policies()
                        .policies()
                        .any(|policy| {
                            policy
                                .id()
                                .to_string()
                                .strip_suffix("#0")
                                .is_some_and(|id| id == lost_policy_id)
                        });
                if revision_applied && route_published && policy_published {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        if convergence.is_err() {
            let applied_revision = listener_cache
                .configuration_revision
                .load(std::sync::atomic::Ordering::SeqCst);
            let route_ids: Vec<_> = listener_router.read().await.route_ids().collect();
            let policy_ids: Vec<_> = listener_authz
                .snapshot()
                .policies()
                .policies()
                .map(|policy| policy.id().to_string())
                .collect();
            panic!(
                "lost-notification convergence timed out: durable={durable_revision_without_notification}, applied={applied_revision}, routes={route_ids:?}, policies={policy_ids:?}"
            );
        }
        assert_eq!(
            listener_cache
                .configuration_revision
                .load(std::sync::atomic::Ordering::SeqCst),
            durable_revision_without_notification
        );
        assert!(listener_cache.session_generation() > generation_before_lost_notification);
        println!(
            "two_gate_fence_check: lost_configuration_notification_reconciles_route_and_policy"
        );
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres, Redis and host orchestration from the RA06c task 2.2 fixture"]
    async fn authority_recovery_requires_snapshot_and_replay() {
        let redis_url = std::env::var("RA06C02_REDIS_URL").expect("RA06C02_REDIS_URL is required");
        let reader_url = std::env::var("RA06C02_ASO_READER_DATABASE_URL")
            .expect("RA06C02_ASO_READER_DATABASE_URL is required");
        let owner_url = std::env::var("RA06C02_ASO_OWNER_DATABASE_URL")
            .expect("RA06C02_ASO_OWNER_DATABASE_URL is required");
        let gate_url = std::env::var("RA06C02_GATE_DATABASE_URL")
            .expect("RA06C02_GATE_DATABASE_URL is required");
        let control_dir = std::path::PathBuf::from(
            std::env::var("RA06C02_REDIS_CONTROL_DIR")
                .expect("RA06C02_REDIS_CONTROL_DIR is required"),
        );
        let source_key = format!("aso-recovery-{}", Uuid::new_v4());

        let source = Arc::new(
            PostgresAuthoritySource::connect(&reader_url, 2)
                .await
                .unwrap(),
        );
        let owner = PgPoolOptions::new()
            .max_connections(1)
            .connect(&owner_url)
            .await
            .unwrap();
        let gate_db = Database::connect(&gate_url, 2).await.unwrap();
        gate_db.migrate().await.unwrap();
        let mut cache = cache(&redis_url).await;
        let fence = Arc::new(
            RedisAuthorityFence::new(
                cache.l2_client.clone().unwrap(),
                cache.l2.clone().unwrap(),
                &source_key,
            )
            .unwrap(),
        );
        let readiness = AuthorityReadiness::new();
        cache.authority_fence = Some(fence.clone());
        cache.authority_readiness = Some(readiness.clone());
        let consumer = AuthorityConsumer::new(
            source_key,
            source.clone(),
            Arc::new(PostgresAuthorityCursorStore::new(gate_db.pool())),
            fence.clone(),
            readiness.clone(),
        );
        consumer.bootstrap().await.unwrap();
        let first = readiness.snapshot().high_water.unwrap();
        let key = AuthoritySessionCacheKey::tenant(
            "kratos",
            "https://identity.example/",
            CredentialKind::Cookie,
            "recovery-credential",
            Uuid::from_u128(51),
        )
        .unwrap();
        let generation = cache.session_generation();
        readiness.install_test_snapshot(first.clone());
        assert!(cache
            .put_versioned_session_if_current(
                &key,
                &identity("recovery-session"),
                &first,
                generation,
            )
            .await
            .unwrap());

        sqlx::query("UPDATE aso.capabilities SET description = description WHERE false")
            .execute(&owner)
            .await
            .unwrap();
        tokio::time::sleep(HIGH_WATER_FRESHNESS + Duration::from_millis(20)).await;
        assert!(!readiness.cache_ready());
        assert!(cache
            .get_versioned_session_if_current(&key, &first, generation)
            .await
            .unwrap()
            .is_none());
        println!("authority_recovery_check: stalled_consumer_expires_cache");

        let observed = source.high_water().await.unwrap();
        let events = source
            .events_after(first.outbox_sequence, observed.outbox_sequence, 256)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, observed.outbox_sequence);
        consumer.synchronize().await.unwrap();
        let advanced = readiness.snapshot().high_water.unwrap();
        assert_eq!(advanced, observed);
        assert!(readiness.cache_ready());
        println!("authority_recovery_check: gap_free_replay_restores_readiness");

        readiness.install_test_snapshot(advanced.clone());
        assert!(cache
            .put_versioned_session_if_current(
                &key,
                &identity("advanced-session"),
                &advanced,
                generation,
            )
            .await
            .unwrap());
        let mut connection = fence.connection.clone();
        let _: usize = redis::cmd("HSET")
            .arg(fence.fence_key())
            .arg("deployment")
            .arg(first.stamp.deployment_id.to_string())
            .arg("incarnation")
            .arg(first.stamp.incarnation.to_string())
            .arg("revision")
            .arg(first.stamp.revision.to_string())
            .arg("sequence")
            .arg(first.outbox_sequence.to_string())
            .query_async(&mut connection)
            .await
            .unwrap();
        readiness.install_test_snapshot(advanced.clone());
        assert!(cache
            .get_versioned_session_if_current(&key, &advanced, generation)
            .await
            .unwrap()
            .is_none());
        assert!(readiness.bootstrap_required());
        println!("authority_recovery_check: restored_old_snapshot_forces_bypass");

        assert!(matches!(
            consumer.synchronize().await,
            Err(AuthorityError::SharedFenceMismatch)
        ));
        assert!(!readiness.cache_ready());
        consumer.bootstrap().await.unwrap();
        assert_eq!(readiness.snapshot().high_water.as_ref(), Some(&advanced));
        assert!(readiness.cache_ready());
        assert!(fence.matches(&advanced).await.unwrap());
        println!("authority_recovery_check: restored_old_snapshot_requires_bootstrap");

        readiness.install_test_snapshot(advanced.clone());
        assert!(cache
            .put_versioned_session_if_current(
                &key,
                &identity("partition-session"),
                &advanced,
                generation,
            )
            .await
            .unwrap());
        std::fs::write(control_dir.join("partition-ready"), b"ready").unwrap();
        for _ in 0..600 {
            if control_dir.join("partitioned").is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(control_dir.join("partitioned").is_file());
        readiness.install_test_snapshot(advanced.clone());
        let partition_started = std::time::Instant::now();
        let partition_result = tokio::time::timeout(
            Duration::from_secs(10),
            cache.get_versioned_session_if_current(&key, &advanced, generation),
        )
        .await
        .expect("Redis partition must be observed within ten seconds")
        .unwrap();
        assert!(partition_result.is_none());
        assert!(partition_started.elapsed() < Duration::from_secs(1));
        assert!(readiness.bootstrap_required());
        std::fs::write(control_dir.join("partition-observed"), b"observed").unwrap();
        println!("authority_recovery_check: redis_partition_forces_bypass");

        for _ in 0..600 {
            if control_dir.join("restored-empty").is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(control_dir.join("restored-empty").is_file());
        let mut empty_fence_observed = false;
        for _ in 0..100 {
            if matches!(fence.observe().await, Ok(SharedAuthorityFence::Missing)) {
                empty_fence_observed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(empty_fence_observed);
        assert!(matches!(
            consumer.synchronize().await,
            Err(AuthorityError::SharedFenceMismatch)
        ));
        assert!(!readiness.cache_ready());
        consumer.bootstrap().await.unwrap();
        assert_eq!(readiness.snapshot().high_water.as_ref(), Some(&advanced));
        assert!(readiness.cache_ready());
        assert!(fence.matches(&advanced).await.unwrap());
        readiness.install_test_snapshot(advanced.clone());
        assert!(cache
            .get_versioned_session_if_current(&key, &advanced, generation)
            .await
            .unwrap()
            .is_none());
        println!("authority_recovery_check: empty_restart_requires_bootstrap");

        let mut connection = fence.connection.clone();
        let _: usize = redis::cmd("DEL")
            .arg(fence.fence_key())
            .arg(fence.session_key(&key, &first))
            .arg(fence.session_key(&key, &advanced))
            .query_async(&mut connection)
            .await
            .unwrap();
    }
}
