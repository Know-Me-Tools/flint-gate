/// Approval store abstraction and in-memory implementation.
///
/// The `ApprovalStore` trait provides the swap point for `MemoryApprovalStore`
/// (single-replica, ephemeral) and `PostgresApprovalStore` (durable, cross-replica).
/// `AppState` holds `Arc<dyn ApprovalStore + Send + Sync>` so the backend can be
/// selected at startup without touching any request-handling code.
pub mod postgres;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ── Public types ──────────────────────────────────────────────────────────────

/// A pending approval request registered by an agent tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: Uuid,
    pub agent_sub: String,
    pub tool_name: String,
    pub reason: String,
    pub registered_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub decision: Option<ApprovalDecision>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// The outcome of a human-approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

/// A lightweight status record returned by `status()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalStatus {
    pub id: Uuid,
    pub decision: Option<ApprovalDecision>,
    pub decided_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Backend-agnostic approval persistence contract.
///
/// Implementations must be `Send + Sync` — they are stored in `Arc<dyn ApprovalStore>`
/// inside `AppState` and accessed concurrently from Axum handlers.
#[async_trait]
pub trait ApprovalStore: Send + Sync {
    /// Register a new pending approval; returns the generated ID.
    async fn register(
        &self,
        agent_sub: &str,
        tool_name: &str,
        reason: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Uuid>;

    /// Record an approval decision (approved or rejected).
    ///
    /// Returns `false` if the approval ID does not exist.
    async fn decide(&self, id: Uuid, decision: ApprovalDecision) -> Result<bool>;

    /// List all pending (undecided, non-expired) approvals.
    async fn list(&self) -> Result<Vec<PendingApproval>>;

    /// Return the status of a single approval by ID.
    ///
    /// Returns `None` if the ID is not found.
    async fn status(&self, id: Uuid) -> Result<Option<ApprovalStatus>>;

    /// Delete all approvals whose `expires_at` is in the past.
    async fn purge_expired(&self) -> Result<u64>;

    /// Return the earliest `expires_at` among all pending approvals,
    /// or `None` if there are none.
    async fn earliest_expiry(&self) -> Result<Option<DateTime<Utc>>>;
}

// ── In-memory implementation ──────────────────────────────────────────────────

/// In-process `DashMap`-backed approval store.
///
/// Data is lost on pod restart. Suitable for single-replica deployments or
/// development; replace with `PostgresApprovalStore` for production HA setups.
#[derive(Default)]
pub struct MemoryApprovalStore {
    entries: DashMap<Uuid, PendingApproval>,
}

impl MemoryApprovalStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: DashMap::new(),
        })
    }
}

#[async_trait]
impl ApprovalStore for MemoryApprovalStore {
    async fn register(
        &self,
        agent_sub: &str,
        tool_name: &str,
        reason: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        self.entries.insert(
            id,
            PendingApproval {
                id,
                agent_sub: agent_sub.to_string(),
                tool_name: tool_name.to_string(),
                reason: reason.to_string(),
                registered_at: now,
                expires_at,
                decision: None,
                decided_at: None,
            },
        );
        Ok(id)
    }

    async fn decide(&self, id: Uuid, decision: ApprovalDecision) -> Result<bool> {
        match self.entries.get_mut(&id) {
            None => Ok(false),
            Some(mut entry) => {
                entry.decision = Some(decision);
                entry.decided_at = Some(Utc::now());
                Ok(true)
            }
        }
    }

    async fn list(&self) -> Result<Vec<PendingApproval>> {
        let now = Utc::now();
        let pending: Vec<PendingApproval> = self
            .entries
            .iter()
            .filter(|e| e.decision.is_none() && e.expires_at > now)
            .map(|e| e.clone())
            .collect();
        Ok(pending)
    }

    async fn status(&self, id: Uuid) -> Result<Option<ApprovalStatus>> {
        Ok(self.entries.get(&id).map(|e| ApprovalStatus {
            id: e.id,
            decision: e.decision,
            decided_at: e.decided_at,
            expires_at: e.expires_at,
        }))
    }

    async fn purge_expired(&self) -> Result<u64> {
        let now = Utc::now();
        let expired: Vec<Uuid> = self
            .entries
            .iter()
            .filter(|e| e.expires_at <= now)
            .map(|e| e.id)
            .collect();
        let count = expired.len() as u64;
        for id in expired {
            self.entries.remove(&id);
        }
        Ok(count)
    }

    async fn earliest_expiry(&self) -> Result<Option<DateTime<Utc>>> {
        let earliest = self
            .entries
            .iter()
            .filter(|e| e.decision.is_none())
            .map(|e| e.expires_at)
            .min();
        Ok(earliest)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn future_expiry() -> DateTime<Utc> {
        Utc::now() + Duration::hours(1)
    }

    fn past_expiry() -> DateTime<Utc> {
        Utc::now() - Duration::seconds(1)
    }

    #[tokio::test]
    async fn register_and_status() {
        let store = MemoryApprovalStore::new();
        let id = store
            .register("agent-1", "send_email", "needs approval", future_expiry())
            .await
            .unwrap();
        let s = store.status(id).await.unwrap().unwrap();
        assert_eq!(s.id, id);
        assert!(s.decision.is_none());
    }

    #[tokio::test]
    async fn decide_approved() {
        let store = MemoryApprovalStore::new();
        let id = store
            .register("agent-1", "send_email", "reason", future_expiry())
            .await
            .unwrap();
        let found = store
            .decide(id, ApprovalDecision::Approved)
            .await
            .unwrap();
        assert!(found);
        let s = store.status(id).await.unwrap().unwrap();
        assert_eq!(s.decision, Some(ApprovalDecision::Approved));
        assert!(s.decided_at.is_some());
    }

    #[tokio::test]
    async fn decide_returns_false_for_unknown_id() {
        let store = MemoryApprovalStore::new();
        let found = store
            .decide(Uuid::new_v4(), ApprovalDecision::Rejected)
            .await
            .unwrap();
        assert!(!found);
    }

    #[tokio::test]
    async fn list_only_returns_pending() {
        let store = MemoryApprovalStore::new();
        let id1 = store
            .register("a", "tool", "r", future_expiry())
            .await
            .unwrap();
        let id2 = store
            .register("b", "tool", "r", future_expiry())
            .await
            .unwrap();
        store
            .decide(id1, ApprovalDecision::Rejected)
            .await
            .unwrap();
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id2);
    }

    #[tokio::test]
    async fn list_excludes_expired() {
        let store = MemoryApprovalStore::new();
        store
            .register("a", "tool", "r", past_expiry())
            .await
            .unwrap();
        let list = store.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn purge_expired_removes_past_entries() {
        let store = MemoryApprovalStore::new();
        store
            .register("a", "tool", "r", past_expiry())
            .await
            .unwrap();
        store
            .register("b", "tool", "r", future_expiry())
            .await
            .unwrap();
        let removed = store.purge_expired().await.unwrap();
        assert_eq!(removed, 1);
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn earliest_expiry_returns_soonest() {
        let store = MemoryApprovalStore::new();
        let soon = Utc::now() + Duration::minutes(10);
        let later = Utc::now() + Duration::hours(2);
        store
            .register("a", "t", "r", later)
            .await
            .unwrap();
        store
            .register("b", "t", "r", soon)
            .await
            .unwrap();
        let earliest = store.earliest_expiry().await.unwrap().unwrap();
        assert!((earliest - soon).num_seconds().abs() < 2);
    }

    #[tokio::test]
    async fn cross_replica_decision_returns_not_found() {
        // A decision for an ID that was never registered must return false.
        let store = MemoryApprovalStore::new();
        let missing_id = Uuid::new_v4();
        let found = store
            .decide(missing_id, ApprovalDecision::Approved)
            .await
            .unwrap();
        assert!(!found, "cross-replica decision on unknown id must return false");
    }
}
