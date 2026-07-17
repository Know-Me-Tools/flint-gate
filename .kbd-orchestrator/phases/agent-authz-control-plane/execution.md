# Execution — agent-authz-control-plane

**Date:** 2026-07-03
**Phase:** agent-authz-control-plane
**Backend:** `openspec` (spec-backed traceability; 8 changes with proposal + tasks each)
**Driver:** `/kbd-apply <change-id>` — the KBD-owned apply driver (ONE task per turn; fires `task:before`/`task:after`, emits the position signal, syncs `progress.json` + waypoint). **Never** drive bare `/opsx:apply` (no KBD awareness).

## Backend selection rationale

OpenSpec is the backend: `openspec/` is present, all 8 changes exist as OpenSpec proposals, and the phase requires spec→code traceability for a security-sensitive authorization control plane. No external tool (Antigravity/Roo/etc.) is pinned for execution (`preferred_execution_agents: []`), so Claude Code drives via `/kbd-apply`.

## Dispatch order (dependency-respecting)

Apply changes strictly in this order — later changes depend on earlier ones:

1. `add-budget-rate-limiting`  (G3) — no deps; first
2. `add-mcp-resource-server`   (G1) — no deps; plumbs identity claims
3. `add-policy-engine`         (G2a) — needs #2 claims
4. `add-per-tool-authz`        (G2b) — needs #3 (+#2)
5. `add-authz-audit-trail`     (G6b/G8) — needs #3
6. `add-hitl-approval`         (G5) — needs #4
7. `add-guardrail-hook`        (G6a) — no deps (may run anytime after #1)
8. `add-web-config-ui`         (G4) — needs #1–#7 surfaced; last

## Per-change contract

For each change, in order:
1. `/kbd-apply <change-id>` — walk its tasks one per turn; KBD fires per-task hooks, updates `progress.json`, emits position signals.
2. On all tasks done → change reaches `DONE` in `progress.json`.
3. **QA gate (artifact-refiner):** `/refine-validate "<change-id>"` against `.kbd-orchestrator/constraints.md`.
   - Skip QA when: <3 files modified, or docs-only, or `--skip-qa`. (Most changes here touch ≥3 files → QA runs.)
   - ALL PASS → `/opsx:verify <change-id>` → `/opsx:archive <change-id>`.
   - ANY FAIL → mark change `BLOCKED` in `progress.json` → `/refine-code "<change-id>"` → re-validate.
4. Advance to the next change.

## Verification per change (from constraints)

Every change's final task asserts the workspace stays green:
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- New features ≥80% covered.

Security-sensitive changes (`add-mcp-resource-server`, `add-policy-engine`, `add-per-tool-authz`, `add-hitl-approval`) additionally get a `security-reviewer` pass before archive.

## Cross-cutting guardrails (enforced during execution)

- Single-binary, no sidecar: embedded Cedar, in-process governor, embedded SPA; Redis stays optional (`redis-l2`).
- Schema via idempotent `migrate()` — new tables `authz_policies`, `authz_audit`, `approvals`.
- Stay on `jsonwebtoken@9` (avoid `jwks`-crate v10 conflict).
- No LLM-ops bundle — guardrails ship as an interface only.
- Per-tool authz + HITL live inside `stream/processor.rs` / `a2ui.rs`.

## First pending change

`add-budget-rate-limiting` → next command `/kbd-apply add-budget-rate-limiting`.

## State

Phase is **execution-ready**. `progress.json` initialized with 8 changes (all `pending`), `changes_completed: 0`. Execution proceeds change-by-change via `/kbd-apply`; this skill writes the contract and does not itself walk tasks.
