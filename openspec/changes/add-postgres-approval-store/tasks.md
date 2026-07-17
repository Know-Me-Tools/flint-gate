# Tasks — add-postgres-approval-store

- [ ] Read existing migrations directory to find correct naming convention and next sequence number
- [ ] Write `migrations/XXXX_pending_approvals.sql` with `pending_approvals` table schema
- [ ] Create `crates/flint-gate-core/src/approval/postgres.rs` with `PostgresApprovalStore` struct
- [ ] Implement `ApprovalStore` trait for `PostgresApprovalStore` (register, decide, list, status, purge_expired, earliest_expiry)
- [ ] Add `pg_notify('approval_decided', id)` call in `decide()` for cross-replica wake-up
- [ ] Export `PostgresApprovalStore` from `crates/flint-gate-core/src/approval/mod.rs`
- [ ] Write unit tests for `PostgresApprovalStore` (mock sqlx pool or test-db feature flag)
- [ ] Run `cargo test --workspace` and confirm passing
