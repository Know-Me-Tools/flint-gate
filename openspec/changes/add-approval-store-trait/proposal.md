# Proposal — add-approval-store-trait

**Phase:** post-beta-hardening
**Goal:** G-1a — Postgres approval store: trait abstraction
**Severity:** HIGH
**Depends on:** —

## Problem

`ApprovalManager` in `crates/flint-gate-core/src/approval/mod.rs` is a
concrete `DashMap`-backed struct threaded directly through `AppState`.
There is no abstraction boundary to swap in a Postgres-backed store
without touching every callsite.

## Scope

- `crates/flint-gate-core/src/approval/mod.rs` — extract `ApprovalStore` trait;
  rename concrete struct to `MemoryApprovalStore`
- `crates/flint-gate-core/src/state.rs` (or equivalent) — change
  `approval_manager` field from concrete to `Arc<dyn ApprovalStore + Send + Sync>`
- All files that construct `AppState` with the approval manager

## Out of scope

- Postgres implementation (Change 4)
- Config wiring (Change 5)

## Acceptance Criteria

- `ApprovalStore` trait has methods: `register`, `decide`, `list`,
  `status`, `purge_expired`, `earliest_expiry`
- `MemoryApprovalStore` implements `ApprovalStore` and passes all
  existing unit tests
- `AppState` holds `Arc<dyn ApprovalStore + Send + Sync>`
- `cargo test --workspace` passes with zero regressions
- The `cross_replica_decision_returns_not_found` test still passes
