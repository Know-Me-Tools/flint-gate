---
type: Reference
id: flint-forge-p3-c019-postgrest-core-merge-and-parity-pass-status
title: Flint Forge p3-c019 PostgREST core merge and parity pass status
tags:
- flint-forge
- auth-rls
- postgrest
- fdb-query
- graphql-subscriptions
- keto
- phase-status
links:
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
- flint-forge-g4-graphql-subscription-wiring-seam
sources:
- stdin
- manual:Flint Forge/p3-auth-rls-keto
timestamp: 2026-07-03T16:28:11.524673+00:00
created_at: 2026-07-03T16:28:11.524673+00:00
updated_at: 2026-07-03T16:28:11.524673+00:00
revision: 0
---

## Phase Context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T16:25:58Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`
- **Current baseline:** `main` at merge commit `8f29382`

## Phase Gate

All authentication and authorization layers must be live end-to-end:

1. A real `flint-gate` JWT causes a real Postgres RLS row filter.
2. A Keto relation check gates mutations.
3. A Cedar policy controls capability-level access.
4. No plaintext credentials appear in any log line or tracing span.
5. CRUD handler bodies execute parameterized SQL.

Broader phase scope is tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md).

## Phase Goals

- **G1 — `forge-policy`: Cedar policy evaluation crate**
  - `PolicyEngine::evaluate(principal, action, resource, context)` returns allow/deny.
  - Policy bundles load from `flint_meta.cedar_policies`.
- **G2 — Keto coarse relationship checks**
  - Enforced at subscribe-time and mutation-time.
  - `KetoCacheClient` caches relation tuples with TTL.
  - Cache invalidates on Keto webhook.
  - Integrated into `fdb-app` use-cases.
- **G3 — Full RLS CRUD handler bodies in `RestCompiler`**
  - Implement `handle_list`, `handle_insert`, `handle_update`, and `handle_delete`.
  - Use parameterized SQL.
  - Support filter operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`, `cs`, `cd`.
  - Support Range header pagination.
  - Validate column names for safety.
- **G4 — GraphQL hybrid**
  - `pg_graphql` passthrough for Query/Mutation under RLS.
  - `async-graphql` `Subscription` over `graphql-transport-ws` pulling from `ChangeStreamSource`.
  - Introspection merges `pg_graphql` schema with subscription SDL.
  - Current G4 seam is open in PR #2 and tracked in [Flint Forge G4 GraphQL subscription wiring seam](/flint-forge-g4-graphql-subscription-wiring-seam.md).
- **G5 — Subscription RLS enforcement**
  - For every `EntityChange` from `fdb-realtime`, re-query the changed row as the subscriber with full `RlsContext` before delivery.
  - This WAL-bypass protection is non-negotiable.
- **G6 — Gate tests**
  - `test_rest_select_with_eq_filter` plus coverage for all 12 filter operators.
  - `test_vault_dek_not_in_compiled_state` for DEK serde security.
  - `test_subscription_rls_drops_unauthorized_events`.
  - `test_keto_check_gates_mutation`.
- **G7 — `fdb-realtime` gRPC client**
  - `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC.
  - Authenticated via service token.
  - Includes reconnect loop and fan-out to subscriber streams.

## Phase 2 Dependencies Already Delivered

- `CompiledState` and `DatabaseModel` — delivered in `p2-c003`.
- `RestCompiler` route registration — delivered in `p2-c004`; handler bodies remain Phase 3 work.
- `StateManager` + `ArcSwap` hot-reload — delivered in `p2-c005`.
- `fdb-auth` JWT verify to `RlsContext` — delivered in `p2-c001`.
- `SET LOCAL` RLS propagation — delivered in `p2-c002`.

## Pre-flight Requirement for G4

Before starting the full GraphQL hybrid work, verify OQ-3 against the PG18 container:

```sql
SELECT extversion FROM pg_extension WHERE extname = 'pg_graphql';
```

If `pg_graphql` is not installed, defer G4 to `p3-c007` with a stub.

## Current Repository State

- `main` is synced and clean.
- `main` is at merge commit `8f29382`.
- `fdb-query` is on `main`, in the workspace, and contains all 8 modules.
- The merged local branch was cleaned up.
- The merged crates check clean.
- The p3-c019 **core** is now the baseline.

## Merged Work

- **PR #1:** dev-management docs.
- **PR #3:** p3-c019 PostgREST engine core:
  - `fdb-query` merged.
  - `PgRest::execute` live.
  - Reflection wired.

## Open Work

- **PR #2:** G4 GraphQL subscription seam — `https://github.com/Know-Me-Tools/flint-forge/pull/2`
  - Awaiting review/merge.
- **p3-c019 parity pass:** still remaining and should start on a fresh branch off updated `main`.

## p3-c019 Remaining Parity Pass

Remaining sequence:

1. **T10 — Resource embedding**
   - FK-join planner from `DatabaseModel` FK metadata.
   - Support `!fk`.
   - Support `!inner`.
   - Support spread syntax.
   - Support nested embedding.
2. **T11 — Full-text search variants**
3. **T12 — Edge-case hardening**

Each stage should have an integration checkpoint.

## Execution Guidance

- Do not assume parallel fan-out for the parity pass.
- User instruction: say **"use a workflow"** to run the parity pass as a parallel multi-agent workflow.
- Otherwise proceed single-threaded when explicitly approved.

## Next Actions

1. Start p3-c019 parity pass on a fresh branch off `main` when directed.
2. Implement T10 resource embedding, then T11 FTS, then T12 edge cases.
3. Review/merge PR #2 separately for the G4 GraphQL subscription seam.

# Citations

1. stdin
2. manual:Flint Forge/p3-auth-rls-keto