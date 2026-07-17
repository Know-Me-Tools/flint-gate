# add-admin-hardening-and-multi-replica-doc

## Goal

G3 + G4: Close the remaining admin hardening gaps identified in the assessment:
(a) multi-replica approval constraint: startup warning + integration test;
(b) no body-size cap on admin write endpoints;
(c) CORS startup warning when admin_listen is non-loopback with no CORS config.

## Scope

### G3 — Multi-replica constraint (Option 3c)

- `crates/flint-gate/src/main.rs` — on startup, if `admin_listen` resolves to
  a non-loopback address (or `REPLICA_COUNT` env var > 1), emit a structured
  WARN: `"admin approval store is in-memory per-replica; a POST
  /approvals/{id}/decision must reach the replica holding the paused stream.
  Configure sticky sessions or a service mesh for multi-replica deployments."`
- `crates/flint-gate-core/src/admin/mod.rs` or integration test file — add an
  integration test `cross_replica_decision_returns_not_found` that creates two
  independent `ApprovalManager` instances, registers an approval in one, tries
  to decide it in the other, and asserts `ApprovalError::NotFound` (not a
  silent-allow). This proves the constraint is machine-verifiable.
- `README.md` — add a multi-replica deployment note to `### Pending Approvals`.

### G4 — Admin hardening (body-size cap + CORS warning)

- `crates/flint-gate-core/src/admin/mod.rs` — apply
  `axum::extract::DefaultBodyLimit::max(64 * 1024)` to the protected router
  (64 KiB cap; covers any realistic Cedar policy or agent-identity payload).
- `crates/flint-gate/src/main.rs` — emit a startup WARN when `admin_listen` is
  non-loopback and no explicit CORS configuration is present: `"admin_listen is
  bound to a non-loopback address with no CORS policy configured; cross-origin
  admin API consumers will be rejected by the browser. Set server.admin_cors to
  suppress this warning."`  No full CORS middleware this phase — the warning
  is sufficient for this phase's scope.
- Tests: body above 64 KiB returns 413; body at/below passes; multi-replica
  test (above).

## Security requirements

- The body-size cap MUST apply only to the protected router, not to `/health`
  / `/ready` / `/metrics`.
- The startup warnings are `warn!()` (non-blocking), not `error!()` — the
  gateway still starts, but the operator is informed.
- The multi-replica integration test MUST assert `NotFound` (constraint proven),
  not assert an allow — it is a proof-of-constraint, not a regression guard
  for a future fix.
