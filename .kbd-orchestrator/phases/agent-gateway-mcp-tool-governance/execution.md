# Execution — agent-gateway-mcp-tool-governance

_Backend: **openspec** · Driver: **/kbd-apply** (one task per turn) · 3 changes._

## Backend selection

OpenSpec is present (`openspec/changes/` holds the 3 proposals from Spec) and the
phase is `backend: openspec`. Spec-backed traceability required (every change
touches an auth/budget fail-closed seam). **Backend = openspec.**

> Task execution runs through **`/kbd-apply`** — never bare `/opsx:apply`
> (KBD-unaware: no hooks, no progress.json, no waypoint). `/kbd-execute` only
> writes this dispatch contract.

## Dispatch contract (per change, in order)

1. `/kbd-apply <change-id>` walks its `tasks.md` one task per turn
   (`begin-task` → implement → `end-task`), syncing `progress.json` + waypoint.
2. After the final task, the **QA gate**: artifact-refiner constraint validation
   + a **separated security review** (author never grades its own fail-open seam)
   → `.refiner/artifacts/<change-id>/refinement_log.md`. All three changes touch
   auth/budget, so the review is warranted on each.
3. On PASS: `openspec archive <change-id> --skip-specs --yes`. On FAIL: BLOCK,
   refine, re-gate.
4. Sync `progress.json` (`changes_completed++`) then advance the waypoint —
   **in separate commands** (pipeline-enforce hook reads the pre-update count).

## Per-change verification gate (all three)

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; **every new auth/budget path has a fail-closed
(deny / spoof-resistant / degrades_to_deny) test**.

## Ordered changes (from plan.md)

| # | change-id | goal | tasks | new deps | key risk (review focus) |
|---|---|---|---|---|---|
| 1 | `add-agent-delegate-classification` | G1 | 5 | none (reuse derived_kind) | spoof → Agent privilege escalation |
| 2 | `add-agent-budget-scope` | G2 | 5 | none (reuse BudgetScope/posture) | fail-open budget bypass; key collision |
| 3 | `add-tool-authz-metrics` | G3+G4 | 5 | none (reuse metrics.rs) | tool-name/credential in labels; proxy-port leak |

## First dispatch

**`/kbd-apply add-agent-delegate-classification`** — the governance linchpin.

## Standing constraints (in force)

Federate any JWKS IdM, Ory reference, never an IdP (load-bearing in change 1's
gateway-side classification). Blocking: no secrets; admin port 4457 not public
(change 3 `/metrics` = admin-port); no broken tests; config priority
CLI>env>YAML unchanged.
