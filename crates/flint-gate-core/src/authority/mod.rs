//! ASO authority snapshot, durable replay, and process-local cache readiness.

mod consumer;
mod postgres;

pub use consumer::AuthorityConsumer;
pub use postgres::{PostgresAuthorityCursorStore, PostgresAuthoritySource};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

/// A successful high-water observation remains usable for at most 250 ms.
pub const HIGH_WATER_FRESHNESS: Duration = Duration::from_millis(250);
/// Probe twice per freshness window so an ordinary query has time to finish.
pub const HIGH_WATER_PROBE_INTERVAL: Duration = Duration::from_millis(125);

#[derive(Debug, Error)]
pub enum AuthorityError {
    #[error("authority database operation failed")]
    Database(#[source] sqlx::Error),
    #[error("authority reader privilege contract is not satisfied")]
    PrivilegeContract,
    #[error("authority source regressed for the current deployment/incarnation")]
    SourceRegression,
    #[error("authority event is invalid: {0}")]
    InvalidEvent(&'static str),
    #[error("durable authority cursor is unavailable")]
    CursorUnavailable,
    #[error("shared authority fence is unavailable")]
    SharedFenceUnavailable,
    #[error("shared authority fence changed during compare-and-set")]
    SharedFenceMismatch,
    #[error("shared authority fence is ahead of the candidate")]
    SharedFenceRegression,
}

impl From<sqlx::Error> for AuthorityError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityStamp {
    pub deployment_id: Uuid,
    pub incarnation: Uuid,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeniedSessionKey {
    pub issuer: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeniedSession {
    pub key: DeniedSessionKey,
    pub retain_until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorityHighWater {
    pub stamp: AuthorityStamp,
    pub outbox_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritySnapshot {
    pub high_water: AuthorityHighWater,
    pub observed_at: DateTime<Utc>,
    pub denied_sessions: Vec<DeniedSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityEventKind {
    MembershipRevision,
    SessionDenied {
        key: DeniedSessionKey,
        retain_until: Option<DateTime<Utc>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityEvent {
    pub event_id: Uuid,
    pub sequence: i64,
    pub stamp: AuthorityStamp,
    pub occurred_at: DateTime<Utc>,
    pub kind: AuthorityEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityCursor {
    pub source_key: String,
    pub high_water: AuthorityHighWater,
}

#[async_trait]
pub trait AuthoritySource: Send + Sync {
    async fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError>;
    async fn high_water(&self) -> Result<AuthorityHighWater, AuthorityError>;
    async fn events_after(
        &self,
        sequence: i64,
        through: i64,
        limit: i64,
    ) -> Result<Vec<AuthorityEvent>, AuthorityError>;
}

#[async_trait]
pub trait AuthorityCursorStore: Send + Sync {
    async fn load(
        &self,
        source_key: &str,
        stamp: &AuthorityStamp,
    ) -> Result<Option<AuthorityCursor>, AuthorityError>;
    async fn store_snapshot(
        &self,
        source_key: &str,
        high_water: &AuthorityHighWater,
    ) -> Result<AuthorityCursor, AuthorityError>;
    async fn advance(
        &self,
        source_key: &str,
        high_water: &AuthorityHighWater,
    ) -> Result<AuthorityCursor, AuthorityError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedAuthorityFence {
    Missing,
    Present(AuthorityHighWater),
}

#[async_trait]
pub trait AuthorityFenceStore: Send + Sync {
    async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError>;
    async fn bootstrap(
        &self,
        observed: &SharedAuthorityFence,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError>;
    async fn advance(
        &self,
        expected: &AuthorityHighWater,
        candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError>;
    async fn matches(&self, candidate: &AuthorityHighWater) -> Result<bool, AuthorityError>;
}

#[derive(Debug, Default)]
pub struct UnavailableAuthorityFence;

#[async_trait]
impl AuthorityFenceStore for UnavailableAuthorityFence {
    async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError> {
        Err(AuthorityError::SharedFenceUnavailable)
    }

    async fn bootstrap(
        &self,
        _observed: &SharedAuthorityFence,
        _candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        Err(AuthorityError::SharedFenceUnavailable)
    }

    async fn advance(
        &self,
        _expected: &AuthorityHighWater,
        _candidate: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        Err(AuthorityError::SharedFenceUnavailable)
    }

    async fn matches(&self, _candidate: &AuthorityHighWater) -> Result<bool, AuthorityError> {
        Err(AuthorityError::SharedFenceUnavailable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityCacheMode {
    Bootstrapping,
    Ready,
    Bypass,
}

#[derive(Debug, Clone)]
pub struct AuthorityReadinessSnapshot {
    pub mode: AuthorityCacheMode,
    pub high_water: Option<AuthorityHighWater>,
    pub observed_high_water: Option<i64>,
}

#[derive(Debug)]
struct ReadinessState {
    mode: AuthorityCacheMode,
    high_water: Option<AuthorityHighWater>,
    observed_high_water: Option<i64>,
    last_probe: Option<Instant>,
    bootstrap_required: bool,
    denied_sessions: HashMap<DeniedSessionKey, DateTime<Utc>>,
}

/// Readiness belongs to one process. A durable cursor or shared Redis value can
/// never change a newly constructed instance from `Bootstrapping` to `Ready`.
#[derive(Debug, Clone)]
pub struct AuthorityReadiness {
    inner: Arc<RwLock<ReadinessState>>,
}

impl Default for AuthorityReadiness {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthorityReadiness {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ReadinessState {
                mode: AuthorityCacheMode::Bootstrapping,
                high_water: None,
                observed_high_water: None,
                last_probe: None,
                bootstrap_required: true,
                denied_sessions: HashMap::new(),
            })),
        }
    }

    pub fn snapshot(&self) -> AuthorityReadinessSnapshot {
        let state = self.inner.read().unwrap_or_else(|error| error.into_inner());
        AuthorityReadinessSnapshot {
            mode: state.mode,
            high_water: state.high_water.clone(),
            observed_high_water: state.observed_high_water,
        }
    }

    pub fn cache_ready(&self) -> bool {
        self.cache_ready_at(Instant::now())
    }

    /// Return the authority version a cache operation may use, from one
    /// readiness-state observation. A caller must re-check after every await.
    pub fn acquire_cache_high_water(&self) -> Option<AuthorityHighWater> {
        self.acquire_cache_high_water_at(Instant::now())
    }

    fn acquire_cache_high_water_at(&self, now: Instant) -> Option<AuthorityHighWater> {
        let state = self.inner.read().unwrap_or_else(|error| error.into_inner());
        readiness_is_current(&state, now)
            .then(|| state.high_water.clone())
            .flatten()
    }

    /// Confirm that a completed cache operation is still covered by the same
    /// fresh authority observation and that its verified session is not denied.
    pub fn still_permits_session(
        &self,
        expected: &AuthorityHighWater,
        issuer: &str,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let state = self.inner.read().unwrap_or_else(|error| error.into_inner());
        readiness_is_current(&state, Instant::now())
            && state.high_water.as_ref() == Some(expected)
            && !state
                .denied_sessions
                .get(&DeniedSessionKey {
                    issuer: issuer.to_owned(),
                    session_id: session_id.to_owned(),
                })
                .is_some_and(|retain_until| *retain_until > now)
    }

    fn cache_ready_at(&self, now: Instant) -> bool {
        let state = self.inner.read().unwrap_or_else(|error| error.into_inner());
        readiness_is_current(&state, now)
    }

    pub fn is_denied(&self, issuer: &str, session_id: &str, now: DateTime<Utc>) -> bool {
        let state = self.inner.read().unwrap_or_else(|error| error.into_inner());
        state
            .denied_sessions
            .get(&DeniedSessionKey {
                issuer: issuer.to_owned(),
                session_id: session_id.to_owned(),
            })
            .is_some_and(|retain_until| *retain_until > now)
    }

    fn apply_snapshot(&self, snapshot: &AuthoritySnapshot) {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.mode = AuthorityCacheMode::Bypass;
        state.high_water = Some(snapshot.high_water.clone());
        state.observed_high_water = Some(snapshot.high_water.outbox_sequence);
        state.last_probe = None;
        state.denied_sessions = snapshot
            .denied_sessions
            .iter()
            .map(|denial| (denial.key.clone(), denial.retain_until))
            .collect();
    }

    fn observe_high_water(&self, observed: &AuthorityHighWater) -> Result<(), AuthorityError> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(applied) = state.high_water.as_ref() {
            if applied.stamp.deployment_id != observed.stamp.deployment_id
                || applied.stamp.incarnation != observed.stamp.incarnation
            {
                state.mode = AuthorityCacheMode::Bypass;
                state.last_probe = None;
                return Err(AuthorityError::SourceRegression);
            }
            if observed.stamp.revision < applied.stamp.revision
                || observed.outbox_sequence < applied.outbox_sequence
            {
                state.mode = AuthorityCacheMode::Bypass;
                state.last_probe = None;
                return Err(AuthorityError::SourceRegression);
            }
        }
        state.observed_high_water = Some(observed.outbox_sequence);
        if state
            .high_water
            .as_ref()
            .is_none_or(|applied| applied.outbox_sequence < observed.outbox_sequence)
        {
            state.mode = AuthorityCacheMode::Bypass;
            state.last_probe = None;
        }
        Ok(())
    }

    fn apply_event(&self, event: &AuthorityEvent) -> Result<(), AuthorityError> {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let Some(applied) = state.high_water.as_mut() else {
            return Err(AuthorityError::InvalidEvent("snapshot is missing"));
        };
        if applied.stamp.deployment_id != event.stamp.deployment_id
            || applied.stamp.incarnation != event.stamp.incarnation
        {
            return Err(AuthorityError::InvalidEvent("event namespace changed"));
        }
        if event.sequence <= applied.outbox_sequence {
            return Ok(());
        }
        if event.stamp.revision < applied.stamp.revision {
            return Err(AuthorityError::InvalidEvent("event revision regressed"));
        }
        applied.stamp.revision = event.stamp.revision;
        applied.outbox_sequence = event.sequence;
        if let AuthorityEventKind::SessionDenied {
            key,
            retain_until: Some(retain_until),
        } = &event.kind
        {
            state.denied_sessions.insert(key.clone(), *retain_until);
        }
        Ok(())
    }

    fn mark_ready(&self, probe: Instant) {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let caught_up = state
            .high_water
            .as_ref()
            .zip(state.observed_high_water)
            .is_some_and(|(applied, observed)| applied.outbox_sequence >= observed);
        if caught_up && !state.bootstrap_required {
            state.mode = AuthorityCacheMode::Ready;
            state.last_probe = Some(probe);
        } else {
            state.mode = AuthorityCacheMode::Bypass;
            state.last_probe = None;
        }
    }

    pub fn mark_bypass(&self) {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.mode = AuthorityCacheMode::Bypass;
        state.last_probe = None;
    }

    /// A Redis failure or fence mismatch invalidates the proof held by this
    /// process. Only a complete ASO snapshot and replay may clear this flag.
    pub fn request_bootstrap(&self) {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.bootstrap_required = true;
        state.mode = AuthorityCacheMode::Bypass;
        state.last_probe = None;
    }

    pub(crate) fn bootstrap_required(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .bootstrap_required
    }

    fn mark_bootstrapped_ready(&self, probe: Instant) {
        let mut state = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let caught_up = state
            .high_water
            .as_ref()
            .zip(state.observed_high_water)
            .is_some_and(|(applied, observed)| applied.outbox_sequence >= observed);
        if caught_up {
            state.bootstrap_required = false;
            state.mode = AuthorityCacheMode::Ready;
            state.last_probe = Some(probe);
        }
    }

    #[cfg(test)]
    pub(crate) fn install_test_snapshot(&self, high_water: AuthorityHighWater) {
        self.apply_snapshot(&AuthoritySnapshot {
            high_water,
            observed_at: Utc::now(),
            denied_sessions: vec![],
        });
        self.mark_bootstrapped_ready(Instant::now());
    }
}

fn readiness_is_current(state: &ReadinessState, now: Instant) -> bool {
    !state.bootstrap_required
        && state.mode == AuthorityCacheMode::Ready
        && state
            .last_probe
            .and_then(|probe| now.checked_duration_since(probe))
            .is_some_and(|age| age <= HIGH_WATER_FRESHNESS)
        && state
            .high_water
            .as_ref()
            .zip(state.observed_high_water)
            .is_some_and(|(applied, observed)| applied.outbox_sequence >= observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn high_water(sequence: i64) -> AuthorityHighWater {
        AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: Uuid::from_u128(1),
                incarnation: Uuid::from_u128(2),
                revision: sequence.max(1),
            },
            outbox_sequence: sequence,
        }
    }

    #[test]
    fn readiness_expires_without_another_probe() {
        let readiness = AuthorityReadiness::new();
        readiness.apply_snapshot(&AuthoritySnapshot {
            high_water: high_water(3),
            observed_at: Utc::now(),
            denied_sessions: vec![],
        });
        let probed_at = Instant::now();
        readiness.mark_bootstrapped_ready(probed_at);

        assert!(readiness.cache_ready_at(probed_at + HIGH_WATER_FRESHNESS));
        assert!(
            !readiness.cache_ready_at(probed_at + HIGH_WATER_FRESHNESS + Duration::from_millis(1))
        );
    }

    #[test]
    fn readiness_is_process_local() {
        let first = AuthorityReadiness::new();
        first.apply_snapshot(&AuthoritySnapshot {
            high_water: high_water(1),
            observed_at: Utc::now(),
            denied_sessions: vec![],
        });
        first.mark_bootstrapped_ready(Instant::now());

        let restarted = AuthorityReadiness::new();

        assert!(first.cache_ready());
        assert!(!restarted.cache_ready());
        assert_eq!(restarted.snapshot().mode, AuthorityCacheMode::Bootstrapping);
    }

    #[test]
    fn lag_and_regression_disable_cache_use() {
        let readiness = AuthorityReadiness::new();
        readiness.apply_snapshot(&AuthoritySnapshot {
            high_water: high_water(4),
            observed_at: Utc::now(),
            denied_sessions: vec![],
        });
        readiness.mark_bootstrapped_ready(Instant::now());

        readiness.observe_high_water(&high_water(5)).unwrap();
        assert!(!readiness.cache_ready());
        assert!(matches!(
            readiness.observe_high_water(&high_water(3)),
            Err(AuthorityError::SourceRegression)
        ));
        assert!(!readiness.cache_ready());
    }

    #[test]
    fn redis_failure_requires_another_complete_bootstrap() {
        let readiness = AuthorityReadiness::new();
        let current = high_water(7);
        readiness.apply_snapshot(&AuthoritySnapshot {
            high_water: current.clone(),
            observed_at: Utc::now(),
            denied_sessions: vec![],
        });
        readiness.mark_bootstrapped_ready(Instant::now());
        assert_eq!(readiness.acquire_cache_high_water(), Some(current));

        readiness.request_bootstrap();
        readiness.mark_ready(Instant::now());

        assert!(readiness.bootstrap_required());
        assert!(readiness.acquire_cache_high_water().is_none());
    }
}
