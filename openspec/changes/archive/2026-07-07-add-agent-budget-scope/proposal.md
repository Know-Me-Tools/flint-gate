# add-agent-budget-scope

**Phase:** agent-gateway-mcp-tool-governance · **Goal:** G2 (build-002)
**Scope:** `crates/flint-gate-core/src/config/types.rs` (`BudgetScope`),
`crates/flint-gate-core/src/ratelimit/mod.rs` (key derivation),
`crates/flint-gate-core/src/middleware/pipeline.rs` (resolution + posture), docs.
**Depends on:** `add-agent-delegate-classification` (so delegated agents are
budgeted).

## Why

Agent spend cannot be independently governed: `BudgetScope` is **`User | Team`
only**, so an agent is accounted as its `User`. Worse, budget resolution is
**deliberately fail-OPEN** (`resolve_budget_usage` returns `0` → allow on any
Redis/DB error, "so a transient blip never hard-blocks live traffic"). For a
**governance** budget an over-budget agent must not slip through on a backend
outage.

## What

1. **Add `BudgetScope::Agent`** to the enum + key derivation (`incr_budget` /
   `get_budget` key prefix), tied to the change-1 classification so a delegated
   agent's spend is counted against its Agent budget, not its User.
2. **Fail-closed posture for agent budgets** — reuse `BackendUnavailablePosture`
   (built last phase). On a budget-backend outage: **Agent → Deny** (fail-closed,
   default); **User/Team → degrade** (return 0 / allow, preserving human-traffic
   availability). The over-budget check (`budget_exceeded` → block) is unchanged;
   only the outage path gains the per-scope posture.

## Non-goals

- Cost-weighted budgets (token-count windows only, as today).
- Changing User/Team budget behavior (they keep the current degrade).

## Fail-closed requirement

An Agent-scoped budget with an unreachable backend must **deny** (not return 0 /
allow). Over-budget Agent → block. A `degrades_to_deny` test on the Agent path;
a User-path-still-degrades test (no regression).

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; the two posture tests above.
