---
type: Reference
id: flint-forge-p3-5-ci-postgres-hardening-plan
title: Flint Forge p3.5 CI Postgres hardening plan
tags:
- flint-forge
- ci-postgres
- pgvector
- pg-graphql
- integration-tests
- fdb-gateway
- postgrest
- phase-plan
links:
- flint-forge-p3-c020-live-postgres-realtime-path-verification
- flint-forge-pr-5-postgrest-resource-embedding-wiring
- flint-forge-p3-c019-parity-embedding-workflow-checkpoint
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
sources:
- stdin
- manual:Flint Forge/p3.5-ci-postgres-hardening
timestamp: 2026-07-03T20:46:06.108772+00:00
created_at: 2026-07-03T20:46:06.108772+00:00
updated_at: 2026-07-03T20:46:06.108772+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3.5-ci-postgres-hardening`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T20:42:03Z`
- **Status:** `plan_ready`
- **Commit:** `563eb00`
- **Backend:** OpenSpec

This phase converts live Postgres verification from manual/ignored runs into CI-gating coverage. It follows the `p3-auth-rls-keto` reflection and handoff recommendations, closing the gap where real-time and REST paths were proven only manually. It also clears pre-existing `fdb-gateway` test debt so `cargo test --workspace` is green and meaningful when a database is available in CI.

## Phase gate

The real-time and REST paths proven manually against live Postgres in p3 become CI-gating, and `fdb-gateway` test debt is cleared:

- `cargo test --workspace` must pass with a database in CI.
- DB-backed tests must run in CI rather than only through manual `--ignored` execution.
- Workspace clippy must pass with warnings denied.

## Goals

- **G1 — CI Postgres service:** Provision PG18 + `pgvector` + `pg_graphql` in CI via `scripts/ci-check.sh` / Dagger, export `DATABASE_URL`, and run DB-backed tests in CI. Resolves **OQ-9**.
- **G2 — Un-ignore live-PG tests:** Remove `#[ignore]` or gate on `DATABASE_URL` presence for:
  - `fdb-realtime/tests/listen_live_pg.rs`
  - `fdb-reflection` pgvector/meta-listener tests
  - new DB-backed embedding REST path coverage for `select=*,child(*)` producing correct nested JSON
  - `PgRest::execute` DB coverage
- **G3 — Fix `fdb-gateway` test debt:**
  - Isolate `keto_sync_config_ignores_non_numeric_env` so it does not flake under parallel `set_var`.
  - Clear `uninlined_format_args` lint in `tests/a2ui_seed_test.rs`.
- **G4 — Workspace clippy gate:** Ensure `cargo clippy --workspace --all-targets -- -D warnings` passes. Current blocker is the `hello-component` example crate's macro-generated `used_underscore_items` lint; allow or annotate it narrowly.
- **G5 — Reconcile p3 bookkeeping:** Record c019 and c020 as delivered, mark c017 superseded by c020, and resolve/re-scope c018 against merged introspection work.

## Ordered change plan

The plan uses the cloud-native principle: **make the pipeline trustworthy before adding stages to it**. A red or test-free CI gate makes downstream verification meaningless, so the sequence first unblocks and greens existing checks, then adds the DB service, then adds DB tests that consume it.

| Order | Change | Goal | Rationale |
|---:|---|---|---|
| 1 | `p35-c001-clippy-unblock-hello-component` | G4 | CI is already red; unblock the gate before relying on it. Mechanical change. |
| 2 | `p35-c002-gateway-test-debt` | G3 | Green non-DB gateway tests so `cargo test` is runnable and flakes are removed. |
| 3 | `p35-c003-ci-postgres-service` | G1 | Add Dagger PG18 + `pgvector` + `pg_graphql` service binding, export `DATABASE_URL`, and run `cargo test` in CI. Resolves OQ-9. |
| 4 | `p35-c004-db-integration-tests` | G2 | Convert live-PG tests to `DATABASE_URL`-gated execution and add missing embedding REST / `PgRest::execute` coverage. Builds on the CI database service. |
| 5 | `p35-c005-p3-bookkeeping-reconcile` | G5 | Docs/state-only reconciliation after implementation gates are in place. |

## Related phase context

- c020 live Postgres realtime verification is the predecessor manual proof for the realtime path; p3.5 turns that verification into CI-gating coverage: [Flint Forge p3-c020 live Postgres realtime path verification](/flint-forge-p3-c020-live-postgres-realtime-path-verification.md).
- The missing embedding REST DB test should cover the PostgREST resource embedding behavior previously wired in [Flint Forge PR #5 PostgREST resource embedding wiring](/flint-forge-pr-5-postgrest-resource-embedding-wiring.md) and the p3-c019 embedding/parity work in [Flint Forge p3-c019 parity embedding workflow checkpoint](/flint-forge-p3-c019-parity-embedding-workflow-checkpoint.md).
- The phase remains downstream of the broader auth/RLS/Keto gate tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md).

## Artifacts written

- `plan.md`
- Five OpenSpec `proposal.md` files
- `handoffs/plan.md`
- `progress.json` updated with:
  - five-change list
  - `plan_complete: true`
  - `T=5`
- Waypoint refreshed:
  - `status: plan_ready`
  - `next_action: /kbd-apply p35-c001`

## Deferred execution decision

**OQ-9 remains open for c003:** decide whether a prebuilt PG18 image bundles both `pgvector` and `pg_graphql`, or whether to build a pinned Dockerfile. This materially affects c003 implementation size and was intentionally scoped into c003 rather than guessed during planning.

## Environment caveat

KBD hooks/stage-gate did not fire because `KBD_ORCHESTRATOR_ROOT` was unset. Durable artifacts were written and pushed; only hook side-effects were skipped.

## Next action

Run:

```text
/kbd-apply p35-c001-clippy-unblock-hello-component
```

Alternative: use workflow execution to fan out independent early changes `p35-c001` and `p35-c002` in parallel. The c003 image decision must be resolved when implementing the CI Postgres service.

# Citations

1. stdin
2. manual:Flint Forge/p3.5-ci-postgres-hardening