# Proposal — add-postgres-approval-store

**Phase:** post-beta-hardening
**Goal:** G-1b — Postgres approval store: implementation + migration
**Severity:** HIGH
**Depends on:** add-approval-store-trait

## Problem

The `ApprovalStore` trait introduced in the prior change has only one
implementation (`MemoryApprovalStore`), which is per-replica. A pod crash
or eviction loses all pending approvals. A Postgres-backed implementation
provides durability and cross-replica correctness.

## Scope

- `crates/flint-gate-core/src/approval/postgres.rs` — new file:
  `PostgresApprovalStore` implementing `ApprovalStore`
- `migrations/XXXX_pending_approvals.sql` — new table:
  `pending_approvals (id UUID PK, agent_sub, tool_name, reason,
  registered_at, expires_at, decision, decided_at)`
- Postgres `LISTEN`/`NOTIFY` integration (`pg_notify`) for cross-replica
  wake-up on `decide()`
- Unit/integration tests for `PostgresApprovalStore`

## Out of scope

- Config wiring to select the backend (Change 5)
- K8s sticky-session cleanup (Change 5)

## Acceptance Criteria

- `cargo test --workspace` passes
- `TestIntegration_ApprovalExpiry` passes against a
  `PostgresApprovalStore`-backed gateway
- An approval registered on one replica can be retrieved by ID via the API
  (cross-replica read correctness)
- `purge_expired()` deletes rows past `expires_at`
- Migration file follows existing naming convention in `migrations/`

## Constraints

- Use `sqlx` (already a project dependency)
- Follow `anyhow` for error paths, `thiserror` for the library error type
- Do not use `unwrap`/`expect` outside tests
