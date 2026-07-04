---
type: Reference
id: flint-forge-p3-c020-live-postgres-realtime-path-verification
title: Flint Forge p3-c020 live Postgres realtime path verification
tags:
- flint-forge
- fdb-realtime
- postgres-listen-notify
- integration-tests
- auth-rls
- keto
- graphql-subscriptions
- phase-status
links:
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
- flint-forge-pr-5-postgrest-resource-embedding-wiring
- flint-forge-g4-graphql-subscription-wiring-seam
sources:
- stdin
- manual:Flint Forge/p3-auth-rls-keto
timestamp: 2026-07-03T18:39:28.883370+00:00
created_at: 2026-07-03T18:39:28.883370+00:00
updated_at: 2026-07-03T18:39:28.883370+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T18:29:48Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`
- **PR:** #6
- **Commit:** `4037835`

This session closes the live-database verification gap for the realtime path in Phase 3. Broader phase scope and gates remain tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md); PR #5 context is tracked in [Flint Forge PR #5 PostgREST resource embedding wiring](/flint-forge-pr-5-postgrest-resource-embedding-wiring.md).

## Phase gate

All authentication and authorization layers must work end-to-end:

1. A real `flint-gate` JWT causes a real Postgres RLS row filter.
2. A Keto relation check gates mutations.
3. A Cedar policy controls capability-level access.
4. Zero plaintext credentials appear in logs or tracing spans.
5. CRUD handler bodies execute parameterized SQL.

## Relevant phase goals

- **G4 — GraphQL hybrid:** `pg_graphql` passthrough for Query/Mutation under RLS plus `async-graphql` subscriptions over `graphql-transport-ws`, pulling from `ChangeStreamSource`; introspection merges `pg_graphql` schema with subscription SDL. Related wiring seam: [Flint Forge G4 GraphQL subscription wiring seam](/flint-forge-g4-graphql-subscription-wiring-seam.md).
- **G5 — Subscription RLS enforcement:** for every `EntityChange` from `fdb-realtime`, re-query the changed row as the subscriber with full `RlsContext` before delivery. This is required WAL-bypass protection.
- **G7 — `fdb-realtime` gRPC/client source:** `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC, authenticates with service token, reconnects, and fans out to subscriber streams.

## Decision and implementation

Implemented `#[ignore]`-gated live-Postgres integration tests for the realtime path. These tests are excluded from normal CI because CI has no database, but can be run locally with `DATABASE_URL=… --ignored`.

### Tests added and run green

1. **Migration/trigger verification**
   - Applies `enable_change_notify`.
   - Performs insert, update, and delete.
   - Verifies emitted `flint_change` payloads are correct.

2. **Full adapter verification**
   - Exercises `ListenChangeSource::watch`.
   - Uses Keto through `wiremock`.
   - Receives a real Postgres `NOTIFY`.
   - Decodes and delivers a real `ChangeEvent`.

## Runtime behavior constraints captured

- Real `LISTEN`/`NOTIFY` behavior cannot be proven by unit tests alone.
- `NOTIFY` fires only on transaction commit.
- The listener must be attached before the write being tested.
- Concurrent idempotent DDL can still race at the catalog level.

## Bug found by live execution

Live Postgres tests exposed a concurrency bug that mock-only testing would not catch:

- Concurrent tests raced while applying idempotent DDL.
- `CREATE SCHEMA IF NOT EXISTS` and `CREATE OR REPLACE FUNCTION` are not atomic under test concurrency.
- Observed failures included:
  - SQLSTATE `23505`
  - `tuple concurrently updated`

The migration itself was validated as correct; the fault was test setup concurrency, not product DDL.

## Fix

Serialized test DDL setup using a Postgres advisory lock:

- Apply DDL under `pg_advisory_lock`.
- Avoid guessing/retrying on specific catalog error codes.
- Preserve correct migration semantics.

## Verification

- Live Postgres ignored tests pass with `DATABASE_URL=… --ignored`.
- Default `cargo test -p fdb-realtime` reports the live tests as ignored, keeping DB-less CI green.
- `clippy --tests -- -D warnings` is clean.
- Changes committed and pushed to PR #6 at `4037835`.

## Follow-up

- Review and merge PR #5: embedding REST wiring.
- Review and merge PR #6: LISTEN change source plus live-Postgres tests.
- After PRs #5 and #6 land on `main`, run `/kbd-reflect` or `/kbd-status` to reconcile phase change inventory `c017`–`c020` and advance Phase 3.
- The `/kbd-reflect` step was intentionally deferred because it should run against merged `main`, not mid-branch.

# Citations

1. stdin
2. manual:Flint Forge/p3-auth-rls-keto