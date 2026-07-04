---
type: Reference
id: flint-forge-p3-c019-parity-embedding-workflow-checkpoint
title: Flint Forge p3-c019 parity embedding workflow checkpoint
tags:
- flint-forge
- auth-rls
- keto
- fdb-query
- postgrest
- graphql-subscriptions
- phase-status
links:
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
- flint-forge-p3-c019-postgrest-core-merge-and-parity-pass-status
- flint-forge-g4-graphql-subscription-wiring-seam
sources:
- stdin
- manual:Flint Forge/p3-auth-rls-keto
timestamp: 2026-07-03T16:58:27.492032+00:00
created_at: 2026-07-03T16:58:27.492032+00:00
updated_at: 2026-07-03T16:58:27.492032+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T16:53:27Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`
- **Branch:** `feat/p3-c019-parity-embedding`, based on freshly merged `main`
- **Current repo state:** nothing committed in this turn

This checkpoint continues the Phase 3 auth/RLS/Keto work tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md) and follows the p3-c019 PostgREST/fdb-query parity work summarized in [Flint Forge p3-c019 PostgREST core merge and parity pass status](/flint-forge-p3-c019-postgrest-core-merge-and-parity-pass-status.md).

## Phase gate

All four authentication and authorization layers must work end-to-end:

1. A real `flint-gate` JWT causes a real Postgres RLS row filter.
2. A Keto relation check gates mutations.
3. A Cedar policy controls capability-level access.
4. Zero plaintext credentials appear in any log line or tracing span.
5. CRUD handler bodies execute parameterized SQL.

## Phase goals

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
  - Dispatch filter operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`, `cs`, `cd`.
  - Support `Range` header pagination.
  - Validate column names for safety.
- **G4 — GraphQL hybrid**
  - `pg_graphql` passthrough for Query/Mutation under RLS.
  - `async-graphql` `Subscription` over `graphql-transport-ws` from `ChangeStreamSource`.
  - Introspection merges `pg_graphql` schema with subscription SDL.
  - PR #2 for the G4 seam still awaits review/merge; details are tracked in [Flint Forge G4 GraphQL subscription wiring seam](/flint-forge-g4-graphql-subscription-wiring-seam.md).
- **G5 — Subscription RLS enforcement**
  - For every `EntityChange` from `fdb-realtime`, re-query the changed row as the subscriber with full `RlsContext` before delivery.
  - This WAL-bypass protection is non-negotiable.
- **G6 — Gate tests**
  - `test_rest_select_with_eq_filter`, extended across all 12 filter operators.
  - `test_vault_dek_not_in_compiled_state` for DEK serde security.
  - `test_subscription_rls_drops_unauthorized_events`.
  - `test_keto_check_gates_mutation`.
- **G7 — `fdb-realtime` gRPC client**
  - `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC.
  - Authenticated via service token.
  - Includes reconnect loop and fan-out to subscriber streams.

## Delivered dependencies from Phase 2

- `CompiledState` and `DatabaseModel` delivered in `p2-c003`.
- `RestCompiler` route registration delivered in `p2-c004`; handler bodies remain Phase 3 deliverables.
- `StateManager` plus `ArcSwap` hot-reload delivered in `p2-c005`.
- `fdb-auth` JWT verification to `RlsContext` delivered in `p2-c001`.
- `SET LOCAL` RLS propagation delivered in `p2-c002`.

## GraphQL pre-flight requirement

Before starting G4 GraphQL hybrid work, verify OQ-3 against the PG18 container:

```sql
SELECT extversion FROM pg_extension WHERE extname = 'pg_graphql';
```

If `pg_graphql` is not installed, defer G4 to `p3-c007` with a stub.

## Active workflow

A background workflow is running as task `wbyk2vy8a`.

- It runs **Design → Implement → Verify** across three parity families in parallel.
- It produces designs, module source, and adversarial reviews as outputs.
- It deliberately does **not** write to the repo or update `lib.rs`, avoiding concurrent-edit conflicts on a pure crate.
- Its SQL correctness claims are advisory until verified against the actual compiler.

Integration must follow the Integration-First discipline:

1. Wait for the workflow outputs.
2. Review 3 designs, 3 module implementations, and adversarial security reviews.
3. Integrate modules sequentially.
4. Apply reviewer-flagged fixes.
5. Run real local verification:
   - `cargo test -p fdb-query`
   - workspace check/clippy as the integration checkpoint.

## Load-bearing design decision

`fdb-query` **must not depend on** `fdb-reflection`; doing so would invert the intended hexagonal layering.

Resource embedding therefore uses a caller-supplied `EmbedSchema` descriptor:

- `fdb-query` consumes `EmbedSchema` without depending on reflection internals.
- `fdb-reflection` maps its `DatabaseModel` onto `EmbedSchema` at the boundary.
- This constraint was included in every agent prompt for the parity workflow.

## Next integration steps

When task `wbyk2vy8a` completes:

1. Review all generated designs and implementations.
2. Integrate into `fdb-query` sequentially:
   - `embed.rs`
   - FTS operator additions
   - edge-case tests
3. Wire required exports and planner/operator integration:
   - `lib.rs`
   - `plan.rs`
   - `operator.rs`
4. Apply all security-review fixes before committing.
5. Verify with `cargo test -p fdb-query` and workspace checks.
6. Separately, review/merge PR #2 for the G4 seam.

# Citations

1. stdin
2. manual:Flint Forge/p3-auth-rls-keto