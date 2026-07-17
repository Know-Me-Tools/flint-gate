---
type: Reference
id: flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
title: Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint
tags:
- flint-forge
- auth-rls
- keto
- cedar-policy
- graphql-subscriptions
- pg-graphql
- phase-status
sources:
- stdin
- manual:Flint Forge/p3-auth-rls-keto
timestamp: 2026-07-03T14:42:00.540312+00:00
created_at: 2026-07-03T14:42:00.540312+00:00
updated_at: 2026-07-03T14:42:00.540312+00:00
revision: 0
---

## Phase context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T14:40:51Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`

## Phase gate

All four authentication and authorization layers must be live end-to-end:

1. A real `flint-gate` JWT causes a real Postgres RLS row filter.
2. A Keto relation check gates mutations.
3. A Cedar policy controls capability-level access.
4. No plaintext credentials appear in logs or tracing spans.
5. CRUD handler bodies execute parameterized SQL.

## Goals

- **G1 — `forge-policy`: Cedar policy evaluation crate**
  - `PolicyEngine::evaluate(principal, action, resource, context)` returns allow/deny.
  - Policy bundles are loaded from `flint_meta.cedar_policies`.
- **G2 — Keto coarse relationship checks**
  - Checks run at subscribe-time and mutation-time.
  - `KetoCacheClient` caches relation tuples with TTL.
  - Cache invalidates on Keto webhook.
  - Integrated into `fdb-app` use-cases.
- **G3 — Full RLS CRUD handler bodies in `RestCompiler`**
  - Implement `handle_list`, `handle_insert`, `handle_update`, and `handle_delete`.
  - SQL must be parameterized.
  - Filter operator dispatch must support: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`, `cs`, `cd`.
  - Support `Range` header pagination.
  - Validate column-name safety.
- **G4 — GraphQL hybrid**
  - pg_graphql passthrough for Query/Mutation under RLS.
  - `async-graphql` `Subscription` over `graphql-transport-ws` pulling from `ChangeStreamSource`.
  - Introspection merges `pg_graphql` schema ∪ subscription SDL.
- **G5 — Subscription RLS enforcement**
  - For every `EntityChange` from `fdb-realtime`, re-query the changed row as the subscriber with full `RlsContext` before delivery.
  - This WAL-bypass protection is non-negotiable.
- **G6 — Gate tests**
  - `test_rest_select_with_eq_filter` including all 12 filter operators.
  - `test_vault_dek_not_in_compiled_state` for DEK serde security.
  - `test_subscription_rls_drops_unauthorized_events`.
  - `test_keto_check_gates_mutation`.
- **G7 — `fdb-realtime` gRPC client**
  - `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC.
  - Authenticated via service token.
  - Includes reconnect loop and fan-out to subscriber streams.

## Dependencies delivered from Phase 2

- `CompiledState` and `DatabaseModel` — delivered in `p2-c003`.
- `RestCompiler` route registration — delivered in `p2-c004`; handler bodies remain a Phase 3 deliverable.
- `StateManager` + `ArcSwap` hot-reload — delivered in `p2-c005`.
- `fdb-auth` JWT verify → `RlsContext` — delivered in `p2-c001`.
- `SET LOCAL` RLS propagation — delivered in `p2-c002`.

## Pre-flight check for G4

Before starting GraphQL hybrid work, verify OQ-3 against the PG18 container:

```sql
SELECT extversion FROM pg_extension WHERE extname = 'pg_graphql';
```

If `pg_graphql` is not installed, defer G4 to `p3-c007` with a stub.

## Repository/session state

- `main` is at merge commit `e5aa3bc`.
- Compile Economy profiles are live in `Cargo.toml`.
- The merged local branch has been cleaned up.
- `main` is synced and clean; the PR-merge task is complete.
- Remaining working-tree changes are expected pre-existing p3 work plus tool-generated wiki pages; they are unrelated to the docs task.

## G4 checkpoint and scope decision needed

G4's live path is blocked on **OQ-FRF-1**: upstream `flint-realtime-fabric` `WatchEntityType` RPC is not available/shippable in this scope.

Two scope decisions are pending before implementing the security-critical GraphQL subscription seam:

1. **Recommended:** treat G4 as **wire-seam-only** for now:
   - connect GraphQL `Subscription` field → `subscribe_rls_filtered`;
   - leave `FabricChangeSource::watch` as the documented OQ-FRF-1 stub.
2. Treat an **in-process Postgres `LISTEN` `ChangeStreamSource`** as a separate future change, not part of G4.

Implementation was deliberately paused to avoid plausible-but-unverified `who: RlsContext` threading through `graphql-transport-ws` connection init. RLS must fail closed and must never fail open.

The detailed plan is captured in scratchpad `g4-seam-plan.md`.

## Planned next implementation after confirmation

After confirming the wire-seam-only scope and deferring in-process `LISTEN`, implement the subscription wiring seam across four areas:

- `GraphQlCompiler` factory parameter.
- `CompiledState` / `do_compile` propagation.
- `StateManager` constructor propagation.
- Gateway factory with fail-closed `on_connection_init` RLS handling.

Verification plan:

- Run one `cargo check` as the first integration checkpoint.
- Current test-wait budget: `0/3` spent.

# Citations

1. stdin
2. manual:Flint Forge/p3-auth-rls-keto