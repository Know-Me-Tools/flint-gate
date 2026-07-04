---
type: Reference
id: flint-forge-g4-graphql-subscription-wiring-seam
title: Flint Forge G4 GraphQL subscription wiring seam
tags:
- flint-forge
- graphql-subscriptions
- auth-rls
- async-graphql
- pg-graphql
- fdb-gateway
- phase-status
links:
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
sources:
- stdin
- manual:Flint Forge/p3-auth-rls-keto
timestamp: 2026-07-03T15:01:23.737486+00:00
created_at: 2026-07-03T15:01:23.737486+00:00
updated_at: 2026-07-03T15:01:23.737486+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T14:53:31Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`

This session advanced **G4 — GraphQL hybrid** for the `p3-auth-rls-keto` phase. The broader phase gate remains tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md): real `flint-gate` JWT propagation into Postgres RLS, Keto mutation gating, Cedar capability policy checks, no plaintext credentials in logs/spans, and parameterized SQL CRUD handlers.

## Phase Goals

- **G1 — `forge-policy`: Cedar policy evaluation crate**
  - `PolicyEngine::evaluate(principal, action, resource, context)` returns allow/deny.
  - Policy bundles load from `flint_meta.cedar_policies`.
- **G2 — Keto coarse relationship check**
  - Enforced at subscribe-time and mutation-time.
  - `KetoCacheClient` caches relation tuples with TTL.
  - Cache invalidated by Keto webhook.
  - Integrated into `fdb-app` use-cases.
- **G3 — Full RLS CRUD handler bodies in `RestCompiler`**
  - `handle_list`, `handle_insert`, `handle_update`, `handle_delete`.
  - Parameterized SQL.
  - Filter operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`, `cs`, `cd`.
  - Range header pagination.
  - Column-name safety validation.
- **G4 — GraphQL hybrid**
  - `pg_graphql` passthrough for Query/Mutation under RLS.
  - `async-graphql` `Subscription` over `graphql-transport-ws` pulling from `ChangeStreamSource`.
  - Introspection merges `pg_graphql` schema with subscription SDL.
- **G5 — Subscription RLS enforcement**
  - For each `EntityChange` from `fdb-realtime`, re-query changed row as the subscriber with full `RlsContext` before delivery.
  - This WAL-bypass protection is non-negotiable.
- **G6 — Gate tests**
  - `test_rest_select_with_eq_filter` covering all 12 filter operators.
  - `test_vault_dek_not_in_compiled_state`.
  - `test_subscription_rls_drops_unauthorized_events`.
  - `test_keto_check_gates_mutation`.
- **G7 — `fdb-realtime` gRPC client**
  - `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC.
  - Authenticated by service token.
  - Reconnect loop.
  - Fan-out to subscriber streams.

## Phase 2 Dependencies

Delivered dependencies from Phase 2:

- `CompiledState` and `DatabaseModel` — `p2-c003`.
- `RestCompiler` route registration — `p2-c004`; handler bodies are Phase 3 work.
- `StateManager` + `ArcSwap` hot reload — `p2-c005`.
- `fdb-auth` JWT verify to `RlsContext` — `p2-c001`.
- `SET LOCAL` RLS propagation — `p2-c002`.

## G4 Pre-flight Requirement

Before starting the GraphQL hybrid, verify OQ-3 against the PG18 container:

```sql
SELECT extversion FROM pg_extension WHERE extname = 'pg_graphql';
```

If `pg_graphql` is not installed, defer G4 to `p3-c007` with a stub.

## Session Result

The **G4 wiring seam is complete** and passes both compile and lint gates.

Verification:

```bash
cargo check --workspace
cargo clippy -p fdb-app -p fdb-reflection -p fdb-gateway -- -D warnings
```

Results:

- `cargo check --workspace` passed cleanly.
- `cargo clippy -p fdb-app -p fdb-reflection -p fdb-gateway -- -D warnings` passed cleanly.
- Test-wait budget consumed: `1/3` for this epoch.
- No commit was made; branch/PR is pending go-ahead.

## Fixes Made

Resolved compile and dependency failures:

- `E0004`: non-exhaustive match.
- `E0433`: unlinked `futures` crate.

## Domain Constraints

### Thread-safe shared state

GraphQL subscription resolver closures can run on any runtime thread. Produced streams must be `Send + 'static`.

Design consequence:

- Shared subscription state is held through `Arc`.
- The subscription factory captures `Arc<Quarry>`.
- Each field clones the `Arc`.
- `Rc` is explicitly avoided because it would violate `Send` and repeat the domain-web “Rc in state” failure mode.

### Fail-closed authentication

Auth must fail closed at every seam. No unauthenticated or unfiltered events may be yielded.

Implemented fail-closed points:

1. No subscription factory available → empty stream.
2. No `RlsContext` in resolver context → error stream.
3. `connection_init` without a valid bearer token → `Err`, rejecting the WebSocket.

`RlsContext` is resolved during `connection_init` and read inside resolvers via:

```rust
ctx.data::<RlsContext>()
```

## Design

The G4 seam uses a `SubStreamFactory` threaded through the compiler, state manager, and gateway:

```text
compiler -> StateManager -> gateway
```

The factory type is an `Arc<dyn Fn ... + Send + Sync>`, allowing subscription fields to construct streams while satisfying async-graphql threading requirements.

Auth flow:

```text
WebSocket connection_init
  -> validate bearer
  -> construct RlsContext
  -> inject into async-graphql connection data
  -> resolver pulls RlsContext from ctx.data::<RlsContext>()
  -> subscription stream is built through SubStreamFactory
```

## Files Changed

| File | Change |
|---|---|
| `fdb-reflection/.../graphql.rs` | Added `SubStreamFactory`; changed `compile(&model, Option<factory>)`; subscription fields call the factory; fields pull `RlsContext` from resolver context; fail-closed behavior; added `table_to_meta` and `table_subscription_spec` helpers. |
| `fdb-reflection/state_manager.rs` | Threaded the subscription factory through state fields, both constructors, and both `do_compile` sites. |
| `fdb-app/src/lib.rs` | Added `Quarry::subscribe_graphql_values`; added `ChangeEvent` to `async_graphql::Value` projection. |
| `fdb-gateway/src/main.rs` | Added `build_subscription_factory` using `Quarry` and adapters; added fail-closed `connection_init_rls`; wired factory into `StateManager` and WebSocket handler. |
| `fdb-gateway/Cargo.toml` | Added `futures` dependency. |

## Remaining Caveats

The chain compiles and is logically connected, but live events do not yet flow because two downstream bodies remain unimplemented. Both were pre-existing and outside the G4 wiring scope.

1. `FabricChangeSource::watch`
   - Currently returns an empty stream.
   - Tracked as **OQ-FRF-1** for upstream FRF RPC work.
2. `PgRest::execute`
   - Currently `todo!()`.
   - This is the RLS re-query executor.
   - The reflection `RestCompiler` path has real handlers, but this standalone adapter still needs implementation.

Runtime impact:

- Since `FabricChangeSource::watch()` yields an empty stream, `subscribe_rls_filtered` never invokes the `todo!()` in `PgRest::execute` today.
- Therefore there is no current runtime panic risk from this path, but subscriptions will not emit live events until both bodies are implemented.

## Next Steps

1. Commit the G4 wiring seam on a branch and open a PR if approved.
2. Implement the split-out in-process Postgres `LISTEN` `ChangeStreamSource` path.
3. Implement `PgRest::execute` so subscription RLS re-query is real.
4. Once both follow-ups land, subscriptions can emit live events while preserving subscriber-specific RLS filtering.

# Citations

1. stdin
2. manual:Flint Forge/p3-auth-rls-keto