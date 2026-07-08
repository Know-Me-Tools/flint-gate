# add-agent-governance-lint

**Phase:** agent-gateway-budget-and-policy-operability · **Goal:** G1 (build-001) + G4
**Scope:** `crates/flint-gate-core/src/config/types.rs` (lint fn + severity flag),
`crates/flint-gate/src/main.rs` (startup consumption), docs.

## Why

The prior phase's debt #3: agent spend can be **silently under-governed** — a
route reachable by an agent principal can leave its `MaxTokenBudget` at a
non-agent `scope` (accounting agent spend in the user keyspace) or omit a per-tool
`Authorize` hook entirely (ungoverned tool calls). Nothing surfaces this today.
Also (debt #2 / G4): a `scope: agent` + `window: lifetime` budget is not
fail-closed and should be refused.

## What

Add `GateConfig::agent_governance_lint() -> Vec<GovernanceFinding>` (pure,
testable) that walks each route, resolves its **effective** auth provider exactly
as the pipeline does (`route.auth` ?? `site.default_auth` → `auth_providers` →
variant), and flags:
1. an **agent-reachable** route (resolved provider is `Jwt` or `Mcp` — JWKS-backed,
   can carry an RFC 8693 `act`/agent token) whose `MaxTokenBudget` hook is
   **non-agent-scoped**;
2. an agent-reachable route with **no `Authorize` hook**;
3. any `MaxTokenBudget` with `scope: agent` + `window: lifetime` (G4 — not
   fail-closeable).

**Severity model:** findings **WARN** by default (non-breaking on upgrade); a new
`server.strict_agent_governance: bool` (off by default) promotes them to a startup
`anyhow::bail!`. Mirrors `admin_auth_posture` / `oauth_exposure_posture` (allow the
loose case by default; refuse only when the operator opts in).

## Non-goals

- Changing runtime authz/budget behavior (this is a **config-time** lint).
- Auto-fixing configs (report only).

## Fail-safe requirement

Default WARN must not break an existing loose config; `strict_agent_governance:
true` must `bail!` on any finding. Each finding type + the strict/non-strict
posture gets a test; a clean config yields zero findings.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; per-finding + strict-vs-warn tests.
