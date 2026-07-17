---
type: Reference
id: flint-forge-pr-5-postgrest-resource-embedding-wiring
title: 'Flint Forge PR #5 PostgREST resource embedding wiring'
tags:
- flint-forge
- postgrest
- resource-embedding
- auth-rls
- keto
- graphql-subscriptions
- fdb-reflection
links:
- flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint
- flint-forge-p3-c019-postgrest-core-merge-and-parity-pass-status
- flint-forge-p3-c019-parity-embedding-workflow-checkpoint
- flint-forge-g4-graphql-subscription-wiring-seam
sources:
- stdin
timestamp: 2026-07-03T17:45:21.631523+00:00
created_at: 2026-07-03T17:45:21.631523+00:00
updated_at: 2026-07-03T17:45:21.631523+00:00
revision: 0
---

## Context

- **Project:** Flint Forge
- **Phase:** `p3-auth-rls-keto`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-forge`
- **Captured:** `2026-07-03T17:38:56Z`
- **Status:** `in_progress`
- **Progress:** changes `7/9`
- **PR:** https://github.com/Know-Me-Tools/flint-forge/pull/5

This checkpoint continues the Phase 3 auth/RLS/Keto work tracked in [Flint Forge p3 auth RLS Keto phase status and G4 scope checkpoint](/flint-forge-p3-auth-rls-keto-phase-status-and-g4-scope-checkpoint.md), following the PostgREST parity work in [Flint Forge p3-c019 PostgREST core merge and parity pass status](/flint-forge-p3-c019-postgrest-core-merge-and-parity-pass-status.md) and the embedding workflow checkpoint in [Flint Forge p3-c019 parity embedding workflow checkpoint](/flint-forge-p3-c019-parity-embedding-workflow-checkpoint.md).

## Phase gate

All authentication and authorization layers must work end-to-end:

1. A real `flint-gate` JWT causes a real Postgres RLS row filter.
2. A Keto relation check gates mutations.
3. A Cedar policy controls capability-level access.
4. Zero plaintext credentials appear in logs or tracing spans.
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
  - Support filter operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `in`, `is`, `cs`, `cd`.
  - Support Range header pagination.
  - Validate column names for safety.
- **G4 — GraphQL hybrid**
  - `pg_graphql` passthrough for Query/Mutation under RLS.
  - `async-graphql` `Subscription` over `graphql-transport-ws` from `ChangeStreamSource`.
  - Introspection merges `pg_graphql` schema with subscription SDL.
  - Before starting G4, verify OQ-3 against the PG18 container:

```sql
SELECT extversion FROM pg_extension WHERE extname = 'pg_graphql';
```

  - If `pg_graphql` is unavailable, defer G4 to `p3-c007` with a stub.
- **G5 — Subscription RLS enforcement**
  - For each `EntityChange` from `fdb-realtime`, re-query the changed row as the subscriber using full `RlsContext` before delivery.
  - This WAL-bypass protection is non-negotiable.
- **G6 — Gate tests**
  - `test_rest_select_with_eq_filter` covering all 12 filter operators.
  - `test_vault_dek_not_in_compiled_state`.
  - `test_subscription_rls_drops_unauthorized_events`.
  - `test_keto_check_gates_mutation`.
- **G7 — `fdb-realtime` gRPC client**
  - `ChangeStreamSource` adapter connects to `flint-realtime-fabric` `WatchEntityType` RPC.
  - Authenticated by service token.
  - Includes reconnect loop and subscriber fan-out.

## Dependencies from Phase 2

- `CompiledState` and `DatabaseModel` delivered in `p2-c003`.
- `RestCompiler` route registration delivered in `p2-c004`; handler bodies are Phase 3 work.
- `StateManager` plus `ArcSwap` hot reload delivered in `p2-c005`.
- `fdb-auth` JWT verification to `RlsContext` delivered in `p2-c001`.
- `SET LOCAL` RLS propagation delivered in `p2-c002`.

## PR #5 implementation

PR #5 implements PostgREST resource embedding end-to-end for the HTTP consumer path. The engine-side support was previously merged in PR #4; PR #5 wires it into REST list handling.

### Design constraints

- Preserve existing tested handler behavior when no embeds are requested.
- Avoid API guesses: adapt to the real `EmbedSchema` API.
- Maintain the PostgREST list endpoint contract:
  - one request maps to one SQL statement;
  - embedded resources are rendered as per-parent JSON;
  - RLS applies to both parent and child rows;
  - the handler delegates SQL construction to the shared translator;
  - the full statement executes under a single `SET LOCAL` RLS context.

### Added components

- **`embed_schema.rs`**
  - Maps `DatabaseModel` to `EmbedSchema`.
  - Produces bidirectional foreign-key edges.
  - Uses deterministic, validated FK names.
- **`handle_list` integration**
  - Adds `build_inner_query` support for embedded resources.
  - Parses `select=`.
  - Routes embed-scoped query parameters.
  - Resolves and renders correlated subselects inside the inner query.
  - Threads bind parameters in SQL-text order.
  - Aliases the parent table for child correlation.
- **No-embed fallback**
  - Preserved byte-for-byte so existing REST and security gate tests remain unchanged.

## SQL behavior proven by tests

The new embed-path unit tests assert generated SQL shape end-to-end:

- parent table is aliased for correlation;
- embedded child rows use a `json_agg` correlated subselect;
- embed-scoped filter values are bound parameters;
- unsafe relations are rejected.

## API adjustment

The actual engine API did **not** expose a `push_fk_to_table` method; it only exposed `new`, `with_table`, and `table`. The implementation therefore accumulates FK edges in a `BTreeMap` and builds the `EmbedSchema` once, matching the real API rather than relying on an assumed incremental mutator.

## Verification

Base Rule #14 verification completed:

```text
cargo test -p fdb-reflection
# 53 lib tests: 4 new embed-path tests + 3 mapper tests + all prior tests

cargo check --workspace
# clean

clippy -D warnings
# clean
```

Integration gates are green.

## Known gap

Tests currently assert generated SQL, but there is no DB-backed test that executes an actual embedded response against live Postgres. This gap is flagged in PR #5.

## Remaining follow-up

- Review and merge PR #5.
- Implement the in-process Postgres `LISTEN` `ChangeStreamSource` as the OQ-FRF-1 workaround so G4 subscriptions emit live events. This is the remaining real-time piece and relates to the GraphQL subscription seam tracked in [Flint Forge G4 GraphQL subscription wiring seam](/flint-forge-g4-graphql-subscription-wiring-seam.md).

# Citations

1. stdin