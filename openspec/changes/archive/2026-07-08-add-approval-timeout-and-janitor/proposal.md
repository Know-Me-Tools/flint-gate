# add-approval-timeout-and-janitor

**Phase:** agent-approval-and-step-up-flows · **Goal:** G3 (build-001 — BUILD FIRST, safety-critical)
**Scope:** `crates/flint-gate-core/src/middleware/pipeline.rs` (paused-stream timeout
arm), `crates/flint-gate/src/main.rs` (purge_expired janitor task),
`crates/flint-gate-core/src/config/types.rs` (approval config block),
`crates/flint-gate-core/src/approval/mod.rs` (TTL from config), docs.
**Depends on:** nothing.

## Why

The human-in-the-loop approval flow pauses a tool call and awaits a decision on
`approval_rx.recv()` (`pipeline.rs:821`) with **no deadline** — an undecided
approval **hangs the paused stream forever** (the TTL is enforced only inside
`decide()`, i.e. only if a decision arrives). `ApprovalManager::purge_expired()`
(`approval/mod.rs:158`) is **defined but never called**, so expired entries leak
from the DashMap. There is no approval config. This is a fail-open-to-hang defect
— the sharpest safety gap in the flow.

## What

1. **Paused-stream timeout → auto-DENY.** Add a `tokio::time::sleep_until` arm to
   the paused-stream `select!` (`pipeline.rs:815-833`), with the deadline set to
   the **nearest pending approval's `expires_at`** (the monotonic `Instant` the
   manager already stores). On fire, resolve the held tool call as **Deny** — emit
   the deny event and resume the stream to a clean termination, **never a silent
   drop and never a half-open stream**. Recompute the deadline after each resolve
   (the next pending approval's expiry becomes the new deadline).
2. **`purge_expired` janitor.** Spawn a background `tokio::time::interval` task
   (mirror the session-watchdog interval at `pipeline.rs:744`) that calls
   `ApprovalManager::purge_expired()` periodically, started in `main.rs` alongside
   the other background tasks.
3. **Approval config.** Add `approval: { enabled: bool (default true), ttl_seconds:
   Option<u64> }` to `config/types.rs` (serde default). `ttl_seconds` overrides the
   hardcoded 300s default. `enabled: false` makes a `RequireApproval` decision
   **fail closed to Deny** (an operator who cannot service approvals denies rather
   than hangs).

## Non-goals

- Cross-replica approval routing — `ApprovalManager` stays in-memory per-replica
  (documented single-replica constraint; the timeout + janitor are per-replica,
  which is correct — each replica denies/reaps its own).
- Per-route approval config (global TTL/enable only this change).
- Changing the pause/resume flow itself (that already works).

## Fail-safe requirement

The timeout MUST **auto-deny** the held call (emit the deny event + resume to
termination), never silent-allow and never leave a half-open stream. `enabled:
false` MUST fail closed to Deny. Tested: an undecided approval times out to deny;
two staggered pending approvals each deny at their own deadline; the janitor reaps
expired entries; `enabled:false` denies a RequireApproval.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; timeout-auto-deny + staggered-deadline + janitor + enabled-false
tests; existing pause/resume tests unregressed. Separated security review
(no silent-allow / no half-open stream on timeout).
