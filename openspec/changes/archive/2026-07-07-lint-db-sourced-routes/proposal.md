# lint-db-sourced-routes

**Phase:** agent-governance-completeness-and-policy-authoring · **Goal:** G1 (build-001 — BUILD FIRST)
**Scope:** `crates/flint-gate-core/src/proxy/router.rs` (surface the merged route set),
`crates/flint-gate/src/main.rs` (lint the merged set at startup),
`crates/flint-gate-core/src/cache/mod.rs` (lint on hot-reload), docs.
**Depends on:** nothing.

## Why

`agent_governance_lint()` walks `self.routes` (YAML) only — its doc comment says
so. Routes merged from the database (`database.override_yaml` → `gate_routes`) and
applied via LISTEN/NOTIFY hot-reload **escape the lint entirely**. A DB-only
agent-reachable route with a non-agent budget / no authorize hook / lifetime-agent
budget passes `strict_agent_governance` at boot and is never surfaced. The merged
`Vec<RouteConfig>` already exists at `router.rs:128` but is built and discarded.

## What

1. **Surface the merged route set.** `Router::from_config_and_db_routes` already
   builds the merged `Vec<RouteConfig>` (`router.rs:128`); expose it (return it, or
   a sibling that returns it) so callers can lint the exact set that becomes the
   live router — single source of truth, no merge recomputation/drift.
2. **Lint the merged set at startup.** In `main.rs`, when `database.override_yaml`
   is honored, lint the merged (YAML+DB) set via the existing
   `agent_governance_lint_routes(&[RouteConfig])` instead of the YAML-only
   `agent_governance_lint()`. Keep the existing severity: WARN each; `bail!` under
   `strict_agent_governance` (safe — pre-serve). Avoid double-WARNing YAML routes
   (lint the merged set once, not YAML + merged).
3. **Lint on hot-reload.** In `rebuild_router_from_db` (`cache/mod.rs:398`, which
   holds both `config` and freshly-loaded `db_routes`), lint the merged set on every
   `"routes"` NOTIFY. **A live process CANNOT `bail!`** — so: **WARN always**; under
   `strict_agent_governance`, **reject the offending route(s) and retain the
   last-good router** (fail-closed analog of `bail!` for a running gateway), loudly
   logged so an operator isn't confused why a route didn't take effect.

## Non-goals

- Changing the lint's finding logic (`GovernanceReason` set is unchanged).
- New route-merge semantics (reuse `from_config_and_db_routes`).
- Terminating a live process on reload (impossible + wrong — retain-last-good).

## Fail-safe requirement

Startup strict → `bail!` (unchanged, pre-serve). Reload strict → reject-and-retain
(never exit); the retained router is the last-good, so a bad hot-reloaded route is
NOT applied (default-safe). Tested: DB-only under-governed route flagged at
startup; reload strict retains last-good + logs; non-strict WARNs and still applies.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; merged-set-lint tests (startup strict bail; reload strict
reject-and-retain; non-strict warn); existing YAML-only lint tests unregressed.
