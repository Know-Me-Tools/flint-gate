---
type: Reference
id: flint-forge-p3-5-c002-gateway-env-flake-fix
title: Flint Forge p3.5 c002 gateway env-flake fix
tags:
- flint-forge
- fdb-gateway
- ci-postgres
- keto-sync
- test-flake
- environment-vars
- integration-tests
links:
- flint-forge-p3-5-ci-postgres-hardening-plan
- flint-forge-p3-c020-live-postgres-realtime-path-verification
- flint-forge-pr-5-postgrest-resource-embedding-wiring
sources:
- stdin
- manual:Flint Forge/p3.5-ci-postgres-hardening
timestamp: 2026-07-03T22:03:01.565853+00:00
created_at: 2026-07-03T22:03:01.565853+00:00
updated_at: 2026-07-03T22:03:01.565853+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3.5-ci-postgres-hardening`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T22:00:03Z`
- **Status:** `in_progress`
- **Progress:** changes `2/5`
- **Completed task:** `p35-c002-gateway-test-debt`
- **Commit:** `83181fb`

This checkpoint implements the G3 gateway-test-debt portion of the broader [Flint Forge p3.5 CI Postgres hardening plan](/flint-forge-p3-5-ci-postgres-hardening-plan.md). The phase converts live Postgres coverage from manual/ignored runs into CI-gating tests and clears pre-existing `fdb-gateway` test debt so `cargo test --workspace` is meaningful with a database in CI.

## Phase goals carried in this checkpoint

- **G1 — CI Postgres service:** Provision PG18 + `pgvector` + `pg_graphql` via `scripts/ci-check.sh` / Dagger, export `DATABASE_URL`, and run DB-backed tests in CI. Resolves **OQ-9**.
- **G2 — Un-ignore live-PG tests:** Remove `#[ignore]` or gate on `DATABASE_URL` for:
  - `fdb-realtime/tests/listen_live_pg.rs`, following the live realtime path proven in [Flint Forge p3-c020 live Postgres realtime path verification](/flint-forge-p3-c020-live-postgres-realtime-path-verification.md).
  - `fdb-reflection` pgvector/meta-listener tests.
  - A new DB-backed REST embedding test for `select=*,child(*)` nested JSON, covering the PostgREST embedding path described in [Flint Forge PR #5 PostgREST resource embedding wiring](/flint-forge-pr-5-postgrest-resource-embedding-wiring.md).
  - `PgRest::execute`.
- **G3 — Fix `fdb-gateway` test debt:** Isolate the `keto_sync` env-var test flake and clear `uninlined_format_args` in `tests/a2ui_seed_test.rs`.
- **G4 — Workspace clippy gate:** `cargo clippy --workspace --all-targets -- -D warnings` must pass; currently expected to need a narrow allow/annotation for the `hello-component` macro-generated `used_underscore_items` lint.
- **G5 — p3 bookkeeping:** Record c019 and c020 as delivered, mark c017 superseded by c020, and resolve/re-scope c018 against merged introspection work.

## c002 outcome: gateway test debt cleared

### Root cause

Three `keto_sync_config_*` tests mutated the shared process environment variable `KETO_SYNC_INTERVAL_SECS` with `set_var` / `remove_var` while tests could run in parallel.

- Environment variables are process-global mutable state.
- Parallel test execution introduced an inherent race.
- The tests also reimplemented the parse inline, so they were not reliably exercising the production path.
- This violated the repository rule against hidden state.

### Fix

Implemented a root-cause fix rather than serializing tests:

- Extracted a pure `resolve_interval(Option<&str>) -> Duration` helper.
- Kept the environment read isolated in the wrapper.
- Rewrote tests to call the pure helper with literal inputs instead of mutating process environment.
- Covered these cases through the production parsing path:
  - numeric value
  - missing value
  - non-numeric value
  - empty value
  - negative value
  - default fallback

### Rationale

The design separates cloud-native configuration ingestion from deterministic parsing logic:

```text
process env read -> wrapper -> resolve_interval(Option<&str>) -> Duration
```

This removes shared mutable process state from tests, eliminates the parallel flake, and makes the tests assert real production behavior instead of duplicated parsing logic.

## Verification

- `cargo test -p fdb-gateway --bins` passed **3/3 consecutive runs**.
- Full `fdb-gateway` suite passed.
- `scripts/ci-check.sh` passed:
  - formatting
  - clippy
  - check
- KBD state and `tasks.md` were updated.
- c002 marked `qa_passed`.

## Notes

- The `uninlined_format_args` part of c002 was already cleared during the c001 gate-unblock work; this checkpoint only needed the env-flake fix.
- Remaining phase work: c003–c005.
- Next recommended task: `p35-c003-ci-postgres-service`, which adds a PG18 + `pgvector` + `pg_graphql` service to the Dagger pipeline, exports `DATABASE_URL`, and adds CI `cargo test` stages.
- **OQ-9 remains open:** decide whether to use a prebuilt image that bundles both required extensions or build a custom Dockerfile.

# Citations

1. stdin
2. manual:Flint Forge/p3.5-ci-postgres-hardening