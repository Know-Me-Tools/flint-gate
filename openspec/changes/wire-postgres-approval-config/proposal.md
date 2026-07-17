# Proposal — wire-postgres-approval-config

**Phase:** post-beta-hardening
**Goal:** G-1c — Postgres approval store: config wiring + K8s cleanup
**Severity:** HIGH
**Depends on:** add-postgres-approval-store

## Problem

`PostgresApprovalStore` exists but is never instantiated. There is no
config key to select between `memory` and `postgres` backends. The K8s
`sessionAffinity: ClientIP` band-aid is still in place.

## Scope

- `config.yaml` (and schema) — add `approval.backend: memory | postgres`
- Configuration loading path — instantiate the correct backend at startup
- `docs/docs/operations.md` — document the new config key and migration steps
- `k8s/service-admin.yaml` — remove or annotate `sessionAffinity: ClientIP`
- `config.test.yaml` — add explicit `approval.backend: memory`

## Out of scope

- The `PostgresApprovalStore` implementation (Change 4)

## Acceptance Criteria

- `FLINT_APPROVAL_BACKEND=postgres` (env) or `approval.backend: postgres`
  (YAML) starts the gateway using `PostgresApprovalStore`
- Neither (or `memory`) uses `MemoryApprovalStore` — no behaviour change
- Configuration priority order (CLI > env > YAML) is preserved; existing
  `config_priority_order` tests pass
- `cargo test --workspace` passes
- `docs/docs/operations.md` documents the config key and migration procedure
- `k8s/service-admin.yaml` band-aid is removed or clearly annotated as
  optional when Postgres backend is active
