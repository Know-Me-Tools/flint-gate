use super::{
    AuthorityCursorStore, AuthorityError, AuthorityFenceStore, AuthorityHighWater,
    AuthorityReadiness, AuthoritySource, HIGH_WATER_PROBE_INTERVAL,
};
use std::{sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const EVENT_PAGE_SIZE: i64 = 256;

pub struct AuthorityConsumer {
    source_key: String,
    source: Arc<dyn AuthoritySource>,
    cursors: Arc<dyn AuthorityCursorStore>,
    fence: Arc<dyn AuthorityFenceStore>,
    readiness: AuthorityReadiness,
}

impl AuthorityConsumer {
    pub fn new(
        source_key: impl Into<String>,
        source: Arc<dyn AuthoritySource>,
        cursors: Arc<dyn AuthorityCursorStore>,
        fence: Arc<dyn AuthorityFenceStore>,
        readiness: AuthorityReadiness,
    ) -> Self {
        Self {
            source_key: source_key.into(),
            source,
            cursors,
            fence,
            readiness,
        }
    }

    pub async fn bootstrap(&self) -> Result<(), AuthorityError> {
        self.readiness.request_bootstrap();
        let shared_before_snapshot = self.fence.observe().await?;
        let snapshot = self.source.snapshot().await?;
        if let Some(cursor) = self
            .cursors
            .load(&self.source_key, &snapshot.high_water.stamp)
            .await?
        {
            let regressed = cursor.high_water.outbox_sequence > snapshot.high_water.outbox_sequence
                || cursor.high_water.stamp.revision > snapshot.high_water.stamp.revision;
            if regressed {
                return Err(AuthorityError::SourceRegression);
            }
        }
        self.readiness.apply_snapshot(&snapshot);
        self.cursors
            .store_snapshot(&self.source_key, &snapshot.high_water)
            .await?;
        let (applied, observed_at) = self.catch_up().await?;
        self.fence
            .bootstrap(&shared_before_snapshot, &applied)
            .await?;
        self.readiness.mark_bootstrapped_ready(observed_at);
        Ok(())
    }

    pub async fn synchronize(&self) -> Result<(), AuthorityError> {
        let expected = self
            .readiness
            .snapshot()
            .high_water
            .ok_or(AuthorityError::InvalidEvent("snapshot is missing"))?;
        let (candidate, observed_at) = self.catch_up().await?;
        if candidate == expected {
            if !self.fence.matches(&candidate).await? {
                return Err(AuthorityError::SharedFenceMismatch);
            }
        } else {
            self.fence.advance(&expected, &candidate).await?;
        }
        self.readiness.mark_ready(observed_at);
        Ok(())
    }

    async fn catch_up(&self) -> Result<(AuthorityHighWater, Instant), AuthorityError> {
        loop {
            let observed_at = Instant::now();
            let observed = self.source.high_water().await?;
            self.readiness.observe_high_water(&observed)?;
            let applied = self
                .readiness
                .snapshot()
                .high_water
                .ok_or(AuthorityError::InvalidEvent("snapshot is missing"))?;
            if applied.outbox_sequence < observed.outbox_sequence {
                self.replay(&applied, &observed).await?;
                continue;
            }
            return Ok((applied, observed_at));
        }
    }

    async fn replay(
        &self,
        applied: &AuthorityHighWater,
        observed: &AuthorityHighWater,
    ) -> Result<(), AuthorityError> {
        let mut sequence = applied.outbox_sequence;
        while sequence < observed.outbox_sequence {
            let events = self
                .source
                .events_after(sequence, observed.outbox_sequence, EVENT_PAGE_SIZE)
                .await?;
            if events.is_empty() {
                return Err(AuthorityError::InvalidEvent(
                    "committed high-water row is missing",
                ));
            }
            for event in events {
                if event.sequence <= sequence || event.sequence > observed.outbox_sequence {
                    return Err(AuthorityError::InvalidEvent("event sequence is unordered"));
                }
                self.readiness.apply_event(&event)?;
                sequence = event.sequence;
                self.cursors
                    .advance(
                        &self.source_key,
                        &AuthorityHighWater {
                            stamp: event.stamp,
                            outbox_sequence: event.sequence,
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn run(self: Arc<Self>, cancellation: CancellationToken) {
        struct BypassOnDrop(AuthorityReadiness);
        impl Drop for BypassOnDrop {
            fn drop(&mut self) {
                self.0.mark_bypass();
            }
        }

        let _guard = BypassOnDrop(self.readiness.clone());
        let mut requires_bootstrap = self.readiness.bootstrap_required();
        let mut ticker = tokio::time::interval(HIGH_WATER_PROBE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    info!(source = %self.source_key, "ASO authority consumer stopped");
                    return;
                }
                _ = ticker.tick() => {
                    let result = if requires_bootstrap || self.readiness.bootstrap_required() {
                        self.bootstrap().await
                    } else {
                        self.synchronize().await
                    };
                    match result {
                        Ok(()) => requires_bootstrap = false,
                        Err(AuthorityError::SourceRegression) => {
                            self.readiness.mark_bypass();
                            requires_bootstrap = true;
                            warn!(source = %self.source_key, "ASO authority source changed or regressed; cache bypass remains active");
                        }
                        Err(error) => {
                            self.readiness.mark_bypass();
                            requires_bootstrap = true;
                            error!(source = %self.source_key, %error, "ASO authority synchronization failed; cache bypass remains active");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::{
        AuthorityCursor, AuthorityEvent, AuthorityEventKind, AuthoritySnapshot, AuthorityStamp,
        SharedAuthorityFence, HIGH_WATER_FRESHNESS,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Clone)]
    struct FakeSource {
        snapshot: AuthoritySnapshot,
        high_water: Arc<Mutex<AuthorityHighWater>>,
        events: Arc<Mutex<Vec<AuthorityEvent>>>,
    }

    #[async_trait]
    impl AuthoritySource for FakeSource {
        async fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError> {
            Ok(self.snapshot.clone())
        }

        async fn high_water(&self) -> Result<AuthorityHighWater, AuthorityError> {
            Ok(self.high_water.lock().unwrap().clone())
        }

        async fn events_after(
            &self,
            sequence: i64,
            through: i64,
            limit: i64,
        ) -> Result<Vec<AuthorityEvent>, AuthorityError> {
            Ok(self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.sequence > sequence && event.sequence <= through)
                .take(limit as usize)
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct FakeCursorStore(Mutex<Option<AuthorityCursor>>);

    #[async_trait]
    impl AuthorityCursorStore for FakeCursorStore {
        async fn load(
            &self,
            _source_key: &str,
            stamp: &AuthorityStamp,
        ) -> Result<Option<AuthorityCursor>, AuthorityError> {
            if self.0.lock().unwrap().as_ref().is_some_and(|cursor| {
                cursor.high_water.stamp.deployment_id != stamp.deployment_id
                    || cursor.high_water.stamp.incarnation != stamp.incarnation
            }) {
                return Ok(None);
            }
            Ok(self.0.lock().unwrap().clone())
        }

        async fn store_snapshot(
            &self,
            source_key: &str,
            high_water: &AuthorityHighWater,
        ) -> Result<AuthorityCursor, AuthorityError> {
            let cursor = AuthorityCursor {
                source_key: source_key.to_owned(),
                high_water: high_water.clone(),
            };
            *self.0.lock().unwrap() = Some(cursor.clone());
            Ok(cursor)
        }

        async fn advance(
            &self,
            source_key: &str,
            high_water: &AuthorityHighWater,
        ) -> Result<AuthorityCursor, AuthorityError> {
            self.store_snapshot(source_key, high_water).await
        }
    }

    #[derive(Default)]
    struct FakeFence(Mutex<Option<SharedAuthorityFence>>);

    impl FakeFence {
        fn state(&self) -> SharedAuthorityFence {
            self.0
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(SharedAuthorityFence::Missing)
        }
    }

    #[async_trait]
    impl AuthorityFenceStore for FakeFence {
        async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError> {
            Ok(self.state())
        }

        async fn bootstrap(
            &self,
            observed: &SharedAuthorityFence,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            let mut state = self.0.lock().unwrap();
            let current = state.clone().unwrap_or(SharedAuthorityFence::Missing);
            if current == SharedAuthorityFence::Present(candidate.clone()) {
                return Ok(());
            }
            if current != *observed {
                return Err(AuthorityError::SharedFenceMismatch);
            }
            *state = Some(SharedAuthorityFence::Present(candidate.clone()));
            Ok(())
        }

        async fn advance(
            &self,
            expected: &AuthorityHighWater,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            let mut state = self.0.lock().unwrap();
            let current = state.clone().unwrap_or(SharedAuthorityFence::Missing);
            if current == SharedAuthorityFence::Present(candidate.clone()) {
                return Ok(());
            }
            if current != SharedAuthorityFence::Present(expected.clone()) {
                return Err(AuthorityError::SharedFenceMismatch);
            }
            if expected.stamp.deployment_id != candidate.stamp.deployment_id
                || expected.stamp.incarnation != candidate.stamp.incarnation
                || candidate.stamp.revision < expected.stamp.revision
                || candidate.outbox_sequence < expected.outbox_sequence
            {
                return Err(AuthorityError::SharedFenceRegression);
            }
            *state = Some(SharedAuthorityFence::Present(candidate.clone()));
            Ok(())
        }

        async fn matches(&self, candidate: &AuthorityHighWater) -> Result<bool, AuthorityError> {
            Ok(self.state() == SharedAuthorityFence::Present(candidate.clone()))
        }
    }

    struct DelayedMatchFence(FakeFence);

    #[async_trait]
    impl AuthorityFenceStore for DelayedMatchFence {
        async fn observe(&self) -> Result<SharedAuthorityFence, AuthorityError> {
            self.0.observe().await
        }

        async fn bootstrap(
            &self,
            observed: &SharedAuthorityFence,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            self.0.bootstrap(observed, candidate).await
        }

        async fn advance(
            &self,
            expected: &AuthorityHighWater,
            candidate: &AuthorityHighWater,
        ) -> Result<(), AuthorityError> {
            self.0.advance(expected, candidate).await
        }

        async fn matches(&self, candidate: &AuthorityHighWater) -> Result<bool, AuthorityError> {
            tokio::time::sleep(HIGH_WATER_FRESHNESS + std::time::Duration::from_millis(20)).await;
            self.0.matches(candidate).await
        }
    }

    struct DelayedHighWaterSource {
        inner: FakeSource,
        delay: std::time::Duration,
    }

    #[async_trait]
    impl AuthoritySource for DelayedHighWaterSource {
        async fn snapshot(&self) -> Result<AuthoritySnapshot, AuthorityError> {
            self.inner.snapshot().await
        }

        async fn high_water(&self) -> Result<AuthorityHighWater, AuthorityError> {
            tokio::time::sleep(self.delay).await;
            self.inner.high_water().await
        }

        async fn events_after(
            &self,
            sequence: i64,
            through: i64,
            limit: i64,
        ) -> Result<Vec<AuthorityEvent>, AuthorityError> {
            self.inner.events_after(sequence, through, limit).await
        }
    }

    fn water(sequence: i64) -> AuthorityHighWater {
        AuthorityHighWater {
            stamp: AuthorityStamp {
                deployment_id: Uuid::from_u128(1),
                incarnation: Uuid::from_u128(2),
                revision: sequence.max(1),
            },
            outbox_sequence: sequence,
        }
    }

    fn source(snapshot_sequence: i64, current_sequence: i64) -> FakeSource {
        FakeSource {
            snapshot: AuthoritySnapshot {
                high_water: water(snapshot_sequence),
                observed_at: Utc::now(),
                denied_sessions: vec![],
            },
            high_water: Arc::new(Mutex::new(water(current_sequence))),
            events: Arc::new(Mutex::new(
                ((snapshot_sequence + 1)..=current_sequence)
                    .map(|sequence| AuthorityEvent {
                        event_id: Uuid::from_u128(sequence as u128 + 10),
                        sequence,
                        stamp: water(sequence).stamp,
                        occurred_at: Utc::now(),
                        kind: AuthorityEventKind::MembershipRevision,
                    })
                    .collect(),
            )),
        }
    }

    #[tokio::test]
    async fn bootstrap_replays_commits_after_the_snapshot_before_readiness() {
        let readiness = AuthorityReadiness::new();
        let cursors = Arc::new(FakeCursorStore::default());
        let fence = Arc::new(FakeFence::default());
        let consumer = AuthorityConsumer::new(
            "aso",
            Arc::new(source(1, 3)),
            cursors.clone(),
            fence.clone(),
            readiness.clone(),
        );

        consumer.bootstrap().await.unwrap();

        assert!(readiness.cache_ready());
        assert_eq!(readiness.snapshot().high_water.unwrap().outbox_sequence, 3);
        assert_eq!(
            cursors
                .0
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .high_water
                .outbox_sequence,
            3
        );
        assert_eq!(fence.state(), SharedAuthorityFence::Present(water(3)));
    }

    #[tokio::test]
    async fn rollback_sequence_gaps_are_valid_when_high_water_is_reached() {
        let source = source(1, 4);
        source.events.lock().unwrap().remove(0);
        let readiness = AuthorityReadiness::new();
        let consumer = AuthorityConsumer::new(
            "aso",
            Arc::new(source),
            Arc::new(FakeCursorStore::default()),
            Arc::new(FakeFence::default()),
            readiness.clone(),
        );

        consumer.bootstrap().await.unwrap();

        assert!(readiness.cache_ready());
        assert_eq!(readiness.snapshot().high_water.unwrap().outbox_sequence, 4);
    }

    #[tokio::test]
    async fn durable_cursor_regression_never_makes_a_new_process_ready() {
        let cursors = Arc::new(FakeCursorStore(Mutex::new(Some(AuthorityCursor {
            source_key: "aso".to_owned(),
            high_water: water(5),
        }))));
        let readiness = AuthorityReadiness::new();
        let consumer = AuthorityConsumer::new(
            "aso",
            Arc::new(source(3, 3)),
            cursors,
            Arc::new(FakeFence::default()),
            readiness.clone(),
        );

        assert!(matches!(
            consumer.bootstrap().await,
            Err(AuthorityError::SourceRegression)
        ));
        assert!(!readiness.cache_ready());
    }

    #[tokio::test]
    async fn slow_fence_check_cannot_renew_an_expired_source_observation() {
        let readiness = AuthorityReadiness::new();
        let consumer = AuthorityConsumer::new(
            "aso",
            Arc::new(source(1, 1)),
            Arc::new(FakeCursorStore::default()),
            Arc::new(DelayedMatchFence(FakeFence::default())),
            readiness.clone(),
        );
        consumer.bootstrap().await.unwrap();

        consumer.synchronize().await.unwrap();

        assert!(!readiness.cache_ready());
    }

    #[tokio::test]
    async fn slow_high_water_query_cannot_renew_an_expired_source_observation() {
        let readiness = AuthorityReadiness::new();
        let consumer = AuthorityConsumer::new(
            "aso",
            Arc::new(DelayedHighWaterSource {
                inner: source(1, 1),
                delay: HIGH_WATER_FRESHNESS + std::time::Duration::from_millis(20),
            }),
            Arc::new(FakeCursorStore::default()),
            Arc::new(FakeFence::default()),
            readiness.clone(),
        );

        consumer.bootstrap().await.unwrap();

        assert!(!readiness.cache_ready());
    }
}
