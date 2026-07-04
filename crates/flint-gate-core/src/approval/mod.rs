//! Human-in-the-loop approval routing.
//!
//! When a Cedar policy evaluates to [`AuthzDecision::RequireApproval`], the
//! streaming processors pause the affected tool call and emit a synthetic
//! `gate:approval_request` event. This module owns the shared state that
//! correlates pending approvals with their originating stream and routes
//! operator decisions back to the right stream task.
//!
//! Design:
//! - One shared [`ApprovalManager`] lives in [`AppState`](crate::middleware::AppState)
//!   and [`AdminState`](crate::admin::AdminState).
//! - Each stream task creates a private notification channel and registers the
//!   sender with the manager when its processor creates a pending approval.
//! - Processors store their own protocol-specific buffered state keyed by
//!   `approval_id`; the manager only knows how to route `approve`/`deny`.
//! - The Admin API `POST /approvals/:id/decision` is the concrete decision
//!   channel; the in-band SSE path delivers the request event to the client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// A decision returned by the human operator or frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Allow the paused operation to proceed.
    Approve,
    /// Block the paused operation.
    Deny,
}

/// Errors returned by [`ApprovalManager::decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    /// No pending approval exists with this id.
    NotFound,
    /// The approval request has already expired.
    Expired,
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalError::NotFound => write!(f, "approval request not found"),
            ApprovalError::Expired => write!(f, "approval request expired"),
        }
    }
}

impl std::error::Error for ApprovalError {}

/// Record stored in the shared approval manager.
struct PendingApproval {
    /// Wall-clock instant after which the request is treated as expired.
    expires_at: Instant,
    /// Channel back to the stream task that owns the buffered call. Each
    /// message carries the approval id so one receiver can serve many pending
    /// approvals from the same stream.
    sender: UnboundedSender<(String, ApprovalDecision)>,
    /// UTC timestamp for API responses.
    expires_at_utc: DateTime<Utc>,
}

/// Shared routing table for pending human approvals.
///
/// Cheap to clone: it is just an `Arc` around a [`dashmap::DashMap`].
#[derive(Clone)]
pub struct ApprovalManager {
    inner: Arc<dashmap::DashMap<String, PendingApproval>>,
}

impl Default for ApprovalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// Register a new pending approval.
    ///
    /// Returns `Ok(())` when the approval id is fresh and the sender is stored.
    /// Duplicate ids are rejected so a programming mistake does not silently
    /// overwrite an in-flight request.
    pub fn register(
        &self,
        approval_id: String,
        expires_at: Instant,
        sender: UnboundedSender<(String, ApprovalDecision)>,
    ) -> Result<(), ApprovalError> {
        if self.inner.contains_key(&approval_id) {
            return Err(ApprovalError::NotFound);
        }
        let ttl = expires_at.saturating_duration_since(Instant::now());
        self.inner.insert(
            approval_id,
            PendingApproval {
                expires_at,
                sender,
                expires_at_utc: Utc::now()
                    + chrono::Duration::from_std(ttl)
                        .unwrap_or_else(|_| chrono::Duration::seconds(0)),
            },
        );
        Ok(())
    }

    /// Apply a decision to a pending approval.
    ///
    /// On success the entry is removed and the decision is sent to the owning
    /// stream task. On failure the entry is left in place for diagnostics
    /// (except expired entries, which are removed).
    pub fn decide(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), ApprovalError> {
        let entry = self
            .inner
            .remove(approval_id)
            .map(|(_, v)| v)
            .ok_or(ApprovalError::NotFound)?;

        if Instant::now() > entry.expires_at {
            return Err(ApprovalError::Expired);
        }

        // An unbounded send is used because the receiver is owned by a live
        // tokio task; if the task has dropped, the stream is already gone and
        // the approval is moot. Swallowing the error is safe.
        let _ = entry.sender.send((approval_id.to_string(), decision));
        Ok(())
    }

    /// Look up a pending approval's metadata without resolving it.
    pub fn status(&self, approval_id: &str) -> Option<ApprovalStatus> {
        self.inner.get(approval_id).map(|entry| ApprovalStatus {
            approval_id: approval_id.to_string(),
            expires_at: entry.expires_at_utc,
            expired: Instant::now() > entry.expires_at,
        })
    }

    /// Remove expired entries and return the count removed.
    ///
    /// Callers should run this periodically (e.g. on every new registration or
    /// from a background janitor). Expired approvals are treated as `deny` by
    /// the stream task itself, so stale entries do not leak permissions.
    pub fn purge_expired(&self) -> usize {
        let now = Instant::now();
        let mut removed = 0;
        self.inner.retain(|_, entry| {
            if now > entry.expires_at {
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }

    /// Number of pending approvals currently tracked.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether no approvals are currently tracked.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Read-only view of a pending approval for the Admin API.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalStatus {
    pub approval_id: String,
    pub expires_at: DateTime<Utc>,
    pub expired: bool,
}

/// Default TTL for pending approvals when not otherwise specified.
pub const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[tokio::test]
    async fn register_and_decide() {
        let manager = ApprovalManager::new();
        let (tx, mut rx) = unbounded_channel();
        let id = "a1".to_string();
        manager
            .register(id.clone(), Instant::now() + Duration::from_secs(60), tx)
            .unwrap();

        manager.decide(&id, ApprovalDecision::Approve).unwrap();
        assert_eq!(rx.recv().await, Some((id, ApprovalDecision::Approve)));
        assert!(manager.inner.is_empty());
    }

    #[tokio::test]
    async fn decide_on_unknown_is_error() {
        let manager = ApprovalManager::new();
        assert_eq!(
            manager.decide("missing", ApprovalDecision::Deny),
            Err(ApprovalError::NotFound)
        );
    }

    #[tokio::test]
    async fn expired_decision_is_rejected() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        let id = "a1".to_string();
        manager
            .register(id.clone(), Instant::now() - Duration::from_secs(1), tx)
            .unwrap();

        assert_eq!(
            manager.decide(&id, ApprovalDecision::Approve),
            Err(ApprovalError::Expired)
        );
        assert!(manager.inner.is_empty());
    }

    #[tokio::test]
    async fn duplicate_registration_rejected() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        let id = "a1".to_string();
        manager
            .register(
                id.clone(),
                Instant::now() + Duration::from_secs(60),
                tx.clone(),
            )
            .unwrap();
        assert_eq!(
            manager.register(id, Instant::now() + Duration::from_secs(60), tx),
            Err(ApprovalError::NotFound)
        );
    }

    #[test]
    fn purge_expired_removes_stale_entries() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        manager
            .register(
                "fresh".to_string(),
                Instant::now() + Duration::from_secs(60),
                tx.clone(),
            )
            .unwrap();
        manager
            .register(
                "stale".to_string(),
                Instant::now() - Duration::from_secs(1),
                tx,
            )
            .unwrap();

        assert_eq!(manager.purge_expired(), 1);
        assert_eq!(manager.len(), 1);
    }
}
