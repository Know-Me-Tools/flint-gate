//! Human-in-the-loop approval routing.
//!
//! When a Cedar policy evaluates to [`AuthzDecision::RequireApproval`], the
//! streaming processors pause the affected tool call and emit a synthetic
//! `gate:approval_request` event. This module owns the shared state that
//! correlates pending approvals with their originating stream and routes
//! operator decisions back to the right stream task.
//!
//! Design:
//! - One shared [`ApprovalManager`] (= [`MemoryApprovalStore`]) lives in
//!   [`AppState`](crate::middleware::AppState) and
//!   [`AdminState`](crate::admin::AdminState).
//! - Each stream task creates a private notification channel and registers the
//!   sender with the manager when its processor creates a pending approval.
//! - Processors store their own protocol-specific buffered state keyed by
//!   `approval_id`; the manager only knows how to route `approve`/`deny`.
//! - The Admin API `POST /approvals/:id/decision` is the concrete decision
//!   channel; the in-band SSE path delivers the request event to the client.
//!
//! Abstraction boundary:
//! - [`ApprovalStore`] is the trait used by `AppState` / `AdminState` so that
//!   a Postgres-backed implementation can be wired in without touching call
//!   sites. Stream processors continue to take [`ApprovalManager`] directly
//!   because they need `Clone` and the synchronous `register` / `earliest_expiry`
//!   surface that is inherently in-process.

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

/// Errors returned by [`ApprovalManager`] methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalError {
    /// No pending approval exists with this id.
    NotFound,
    /// The approval request has already expired.
    Expired,
    /// The pending-approval table has reached its configured capacity.
    /// New registrations are denied fail-closed until the janitor or
    /// `decide` calls reclaim entries.
    CapExceeded,
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalError::NotFound => write!(f, "approval request not found"),
            ApprovalError::Expired => write!(f, "approval request expired"),
            ApprovalError::CapExceeded => write!(f, "approval table at capacity"),
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
    /// Principal that originated the paused tool call.
    principal_id: String,
    /// Action (tool name / intent) being authorized.
    action: String,
    /// Resource identifier from the Cedar context.
    resource_id: String,
    /// Human-readable reason from the policy annotation, if any.
    reason: Option<String>,
}

/// Shared abstraction over the pending-approval store.
///
/// Implemented by [`MemoryApprovalStore`] (the default, in-process `DashMap`
/// backend) and, in a later change, by a Postgres-backed store for
/// cross-replica durability.
///
/// Only the Admin API handler surface (`list`, `status`, `decide`) and the
/// background janitor (`purge_expired`) go through this trait. Stream
/// processors take the concrete [`ApprovalManager`] directly because they
/// need `Clone` and the synchronous `register` / `earliest_expiry` path.
pub trait ApprovalStore: Send + Sync {
    /// List all non-expired pending approvals.
    fn list(&self) -> Vec<ApprovalStatus>;
    /// Return a single approval's read-only metadata, or `None` if absent.
    fn status(&self, approval_id: &str) -> Option<ApprovalStatus>;
    /// Apply a human decision to a pending approval, routing it back to the
    /// waiting stream task.
    fn decide(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), ApprovalError>;
    /// Remove expired entries. Returns the count removed.
    fn purge_expired(&self) -> usize;
    /// The earliest expiry `Instant` among the given pending approval ids.
    fn earliest_expiry(&self, ids: &[String]) -> Option<std::time::Instant>;
}

/// Shared routing table for pending human approvals.
///
/// Cheap to clone: it is just an `Arc` around a [`dashmap::DashMap`] and a
/// cap sentinel. The cap is read-only after construction and shared by all clones.
///
/// Implements [`ApprovalStore`] — use `Arc<MemoryApprovalStore>` where a
/// `dyn ApprovalStore` is expected.
#[derive(Clone)]
pub struct MemoryApprovalStore {
    inner: Arc<dashmap::DashMap<String, PendingApproval>>,
    /// Maximum concurrent pending approvals. `None` means unbounded (not
    /// recommended for production). When `Some(n)` and `inner.len() >= n`,
    /// `register()` returns `Err(ApprovalError::CapExceeded)`.
    cap: Option<usize>,
}

/// Backward-compatible type alias — all existing call sites continue to work.
pub type ApprovalManager = MemoryApprovalStore;

impl Default for MemoryApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryApprovalStore {
    /// Create a new manager with no cap (unbounded). Prefer [`with_cap`](Self::with_cap)
    /// for production use.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(dashmap::DashMap::new()),
            cap: None,
        }
    }

    /// Create a manager with the given capacity cap.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            inner: Arc::new(dashmap::DashMap::new()),
            cap: Some(cap),
        }
    }

    /// Register a new pending approval.
    ///
    /// Returns `Ok(())` when the approval id is fresh and the sender is stored.
    /// Duplicate ids are rejected so a programming mistake does not silently
    /// overwrite an in-flight request.
    ///
    /// The `context_meta` tuple carries the display fields stored alongside the
    /// entry and surfaced by [`list`](Self::list) and [`status`](Self::status):
    /// `(principal_id, action, resource_id, reason)`.
    pub fn register(
        &self,
        approval_id: String,
        expires_at: Instant,
        sender: UnboundedSender<(String, ApprovalDecision)>,
        context_meta: (&str, &str, &str, Option<String>),
    ) -> Result<(), ApprovalError> {
        if let Some(cap) = self.cap {
            if self.inner.len() >= cap {
                return Err(ApprovalError::CapExceeded);
            }
        }
        if self.inner.contains_key(&approval_id) {
            return Err(ApprovalError::NotFound);
        }
        let ttl = expires_at.saturating_duration_since(Instant::now());
        let (principal_id, action, resource_id, reason) = context_meta;
        self.inner.insert(
            approval_id,
            PendingApproval {
                expires_at,
                sender,
                expires_at_utc: Utc::now()
                    + chrono::Duration::from_std(ttl)
                        .unwrap_or_else(|_| chrono::Duration::seconds(0)),
                principal_id: principal_id.to_string(),
                action: action.to_string(),
                resource_id: resource_id.to_string(),
                reason,
            },
        );
        Ok(())
    }

    /// Apply a decision to a pending approval.
    ///
    /// On success the entry is removed and the decision is sent to the owning
    /// stream task. On failure the entry is left in place for diagnostics.
    ///
    /// Expiry is checked **before** removal so that:
    /// - A concurrent or retry call on an expired entry consistently gets
    ///   `ApprovalError::Expired` (→ 410) rather than `ApprovalError::NotFound`
    ///   (→ 404) after the first caller already removed it.
    /// - The entry remains visible via `GET /approvals/{id}` (with `expired: true`)
    ///   until the janitor or a subsequent `decide` call removes it.
    pub fn decide(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), ApprovalError> {
        // Peek first to give a stable error for expired entries before touching
        // the map. The subsequent remove is not atomic with this peek, but the
        // only consequence of the race is that two concurrent callers both see
        // `Ok(())` — one's channel send is dropped because the receiver is gone
        // (the task consumed the first decision). No double-approval is possible:
        // the stream task's select! arm is oneshot and the unbounded_send is
        // fire-and-forget.
        {
            let entry = self.inner.get(approval_id).ok_or(ApprovalError::NotFound)?;
            if Instant::now() > entry.expires_at {
                return Err(ApprovalError::Expired);
            }
        }

        let entry = match self.inner.remove(approval_id).map(|(_, v)| v) {
            Some(e) => e,
            None => return Err(ApprovalError::NotFound),
        };

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
            principal_id: entry.principal_id.clone(),
            action: entry.action.clone(),
            resource_id: entry.resource_id.clone(),
            reason: entry.reason.clone(),
            expires_at: entry.expires_at_utc,
            expired: Instant::now() > entry.expires_at,
        })
    }

    /// List all pending approvals, skipping entries whose monotonic deadline
    /// has already elapsed. Expired entries are not removed here — the janitor
    /// or the next `decide` call handles cleanup.
    pub fn list(&self) -> Vec<ApprovalStatus> {
        let now = Instant::now();
        self.inner
            .iter()
            .filter(|entry| now <= entry.expires_at)
            .map(|entry| ApprovalStatus {
                approval_id: entry.key().clone(),
                principal_id: entry.principal_id.clone(),
                action: entry.action.clone(),
                resource_id: entry.resource_id.clone(),
                reason: entry.reason.clone(),
                expires_at: entry.expires_at_utc,
                expired: false,
            })
            .collect()
    }

    /// The earliest `expires_at` (monotonic `Instant`) among the given pending
    /// approval ids, or `None` when none are still tracked. The paused-stream task
    /// uses this as its timeout deadline: when it fires, the still-pending held
    /// calls are auto-denied (fail-closed) so a stream never hangs forever on an
    /// undecided approval.
    pub fn earliest_expiry(&self, ids: &[String]) -> Option<Instant> {
        ids.iter()
            .filter_map(|id| self.inner.get(id).map(|e| e.expires_at))
            .min()
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

impl ApprovalStore for MemoryApprovalStore {
    fn list(&self) -> Vec<ApprovalStatus> {
        MemoryApprovalStore::list(self)
    }

    fn status(&self, approval_id: &str) -> Option<ApprovalStatus> {
        MemoryApprovalStore::status(self, approval_id)
    }

    fn decide(&self, approval_id: &str, decision: ApprovalDecision) -> Result<(), ApprovalError> {
        MemoryApprovalStore::decide(self, approval_id, decision)
    }

    fn purge_expired(&self) -> usize {
        MemoryApprovalStore::purge_expired(self)
    }

    fn earliest_expiry(&self, ids: &[String]) -> Option<std::time::Instant> {
        MemoryApprovalStore::earliest_expiry(self, ids)
    }
}

/// Read-only view of a pending approval for the Admin API.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalStatus {
    pub approval_id: String,
    pub principal_id: String,
    pub action: String,
    pub resource_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub expired: bool,
}

/// Default TTL for pending approvals when not otherwise specified.
pub const DEFAULT_APPROVAL_TTL: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn meta() -> (&'static str, &'static str, &'static str, Option<String>) {
        ("principal-1", "tool:test", "resource-1", None)
    }

    #[tokio::test]
    async fn register_and_decide() {
        let manager = ApprovalManager::new();
        let (tx, mut rx) = unbounded_channel();
        let id = "a1".to_string();
        manager
            .register(id.clone(), Instant::now() + Duration::from_secs(60), tx, meta())
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
            .register(id.clone(), Instant::now() - Duration::from_secs(1), tx, meta())
            .unwrap();

        assert_eq!(
            manager.decide(&id, ApprovalDecision::Approve),
            Err(ApprovalError::Expired)
        );
        // The entry is NOT removed on expiry — peek-before-remove ensures a
        // retry caller gets 410 (Expired) not 404 (NotFound). The janitor or a
        // subsequent purge_expired() call is responsible for cleanup.
        assert_eq!(manager.len(), 1, "expired entry stays until janitor removes it");
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
                meta(),
            )
            .unwrap();
        assert_eq!(
            manager.register(id, Instant::now() + Duration::from_secs(60), tx, meta()),
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
                meta(),
            )
            .unwrap();
        manager
            .register(
                "stale".to_string(),
                Instant::now() - Duration::from_secs(1),
                tx,
                meta(),
            )
            .unwrap();

        assert_eq!(manager.purge_expired(), 1);
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn list_returns_pending_and_skips_expired() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        manager
            .register(
                "live".to_string(),
                Instant::now() + Duration::from_secs(60),
                tx.clone(),
                ("user-1", "tool:bash", "agent-1", Some("sensitive cmd".to_string())),
            )
            .unwrap();
        manager
            .register(
                "dead".to_string(),
                Instant::now() - Duration::from_secs(1),
                tx,
                meta(),
            )
            .unwrap();

        let listed = manager.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].approval_id, "live");
        assert_eq!(listed[0].principal_id, "user-1");
        assert_eq!(listed[0].action, "tool:bash");
        assert_eq!(listed[0].reason.as_deref(), Some("sensitive cmd"));
        assert!(!listed[0].expired);
    }

    #[test]
    fn list_returns_empty_when_all_expired() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        manager
            .register(
                "dead".to_string(),
                Instant::now() - Duration::from_secs(1),
                tx,
                meta(),
            )
            .unwrap();
        assert!(manager.list().is_empty());
    }

    #[test]
    fn status_includes_context_meta() {
        let manager = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        manager
            .register(
                "a1".to_string(),
                Instant::now() + Duration::from_secs(60),
                tx,
                ("alice", "tool:read_file", "fs:/etc/hosts", Some("needs review".to_string())),
            )
            .unwrap();
        let s = manager.status("a1").unwrap();
        assert_eq!(s.principal_id, "alice");
        assert_eq!(s.action, "tool:read_file");
        assert_eq!(s.resource_id, "fs:/etc/hosts");
        assert_eq!(s.reason.as_deref(), Some("needs review"));
        assert!(!s.expired);
    }

    #[test]
    fn earliest_expiry_picks_the_nearest_pending() {
        // Two staggered pending approvals → the paused-stream deadline is the
        // NEAREST expiry, so each denies at its own deadline in turn.
        let manager = ApprovalManager::new();
        let near = Instant::now() + Duration::from_secs(30);
        let far = Instant::now() + Duration::from_secs(120);
        manager
            .register("far".into(), far, unbounded_channel().0, meta())
            .unwrap();
        manager
            .register("near".into(), near, unbounded_channel().0, meta())
            .unwrap();
        let earliest = manager
            .earliest_expiry(&["far".to_string(), "near".to_string()])
            .expect("some pending");
        // The nearest is `near` (30s), not `far` (120s).
        assert!(earliest <= near);
        assert!(earliest < far);
    }

    #[test]
    fn earliest_expiry_none_when_no_pending() {
        let manager = ApprovalManager::new();
        assert!(manager
            .earliest_expiry(&["absent".to_string()])
            .is_none());
        assert!(manager.earliest_expiry(&[]).is_none());
    }

    #[test]
    fn earliest_expiry_ignores_unknown_ids() {
        let manager = ApprovalManager::new();
        let exp = Instant::now() + Duration::from_secs(45);
        manager
            .register("known".into(), exp, unbounded_channel().0, meta())
            .unwrap();
        // A mix of known + unknown ids yields the known one's expiry.
        let earliest = manager
            .earliest_expiry(&["unknown".to_string(), "known".to_string()])
            .expect("known is pending");
        assert!(earliest <= exp);
    }

    // ── Cap tests ─────────────────────────────────────────────────────────────

    #[test]
    fn register_at_cap_returns_cap_exceeded() {
        let manager = ApprovalManager::with_cap(2);
        let ttl = Instant::now() + Duration::from_secs(60);

        manager
            .register("id-1".into(), ttl, unbounded_channel().0, meta())
            .unwrap();
        manager
            .register("id-2".into(), ttl, unbounded_channel().0, meta())
            .unwrap();

        let err = manager.register("id-3".into(), ttl, unbounded_channel().0, meta());
        assert_eq!(err, Err(ApprovalError::CapExceeded), "third register must fail at cap=2");
    }

    // ── Multi-replica isolation test ──────────────────────────────────────────

    #[test]
    fn cross_replica_decision_returns_not_found() {
        // Two independent ApprovalManager instances simulate two replicas. An
        // approval registered on replica-A is invisible to replica-B — decide()
        // on B returns NotFound, proving the in-memory per-replica constraint.
        let replica_a = ApprovalManager::new();
        let replica_b = ApprovalManager::new();

        let ttl = Instant::now() + Duration::from_secs(60);
        replica_a
            .register("req-1".into(), ttl, unbounded_channel().0, meta())
            .expect("register on replica-A must succeed");

        // Replica-B has no knowledge of req-1.
        assert_eq!(
            replica_b.decide("req-1", ApprovalDecision::Approve),
            Err(ApprovalError::NotFound),
            "decision on the wrong replica must return NotFound (in-memory isolation)"
        );
        // Replica-A still holds the entry.
        assert_eq!(replica_a.len(), 1, "replica-A must still hold the pending approval");
    }

    #[test]
    fn second_register_when_under_cap_succeeds() {
        let manager = ApprovalManager::with_cap(5);
        let ttl = Instant::now() + Duration::from_secs(60);

        for i in 0..4 {
            manager
                .register(format!("id-{i}"), ttl, unbounded_channel().0, meta())
                .expect("must succeed under cap");
        }
        assert_eq!(manager.len(), 4);
        // One more is still under cap
        manager
            .register("id-4".into(), ttl, unbounded_channel().0, meta())
            .expect("fifth register must succeed (cap=5, not yet full)");
        assert_eq!(manager.len(), 5);
    }
}
