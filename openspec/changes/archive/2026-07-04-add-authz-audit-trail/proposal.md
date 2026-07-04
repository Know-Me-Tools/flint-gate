# add-authz-audit-trail

## Summary
Record every authorization decision (allow/deny/step-up/approval) to a queryable audit table, surfaced via the Admin API. (Goals G6b/G8)

## Design
New `authz_audit` Postgres table via the idempotent `migrate()` pattern: principal, action, resource, decision, reason, request_id, timestamp. Write from the authz decision path (route-level and per-tool). Add a paged, filterable Admin API read endpoint. Keep writes off the hot path where possible (async/best-effort insert; never block a decision on audit I/O).

Library: none (new table + write path).

## Depends on
- add-policy-engine (decision path to hook the audit write into)

## Scope
IN: authz_audit table, decision-path write, Admin API read (paged/filterable), async non-blocking insert. OUT: UI rendering (add-web-config-ui consumes this endpoint).

## Tasks
- [ ] `authz_audit` table via `migrate()`
- [ ] Write allow/deny/step-up/approval from the authz decision path (async, non-blocking)
- [ ] Admin API read endpoint (paged, filter by principal/decision/time)
- [ ] Tests: decision recorded, read/filter; ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
