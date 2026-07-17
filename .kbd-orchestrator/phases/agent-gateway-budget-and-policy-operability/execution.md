# Execution — agent-gateway-budget-and-policy-operability

_Backend: **openspec** · Driver: **/kbd-apply** (one task per turn) · 3 changes._

## Backend selection

OpenSpec is present (`openspec/changes/` holds the 3 proposals from Spec) and the
phase is `backend: openspec`. Spec-backed traceability required (changes 1–2 alter
startup/deny behavior). **Backend = openspec.**

> Task execution runs through **`/kbd-apply`** — never bare `/opsx:apply`
> (KBD-unaware). `/kbd-execute` only writes this dispatch contract.

## Dispatch contract (per change, in order)

1. `/kbd-apply <change-id>` walks its `tasks.md` one task per turn
   (`begin-task` → implement → `end-task`), syncing `progress.json` + waypoint.
2. After the final task, the **QA gate**: artifact-refiner constraint validation +
   a **separated security review** (author never grades its own seam) →
   `.refiner/artifacts/<change-id>/refinement_log.md`. Full review on changes 1+2
   (startup/deny behavior); light-touch on 3 (validated by the Cedar validator).
3. On PASS: `openspec archive <change-id> --skip-specs --yes`. On FAIL: BLOCK,
   refine, re-gate.
4. Sync `progress.json` (`changes_completed++`) then advance the waypoint —
   **in separate commands** (pipeline-enforce hook reads the pre-update count).

## Per-change verification gate (all three)

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; **every new lint/posture/sugar path has a fail-safe-default
/ fail-closed test**.

## Ordered changes (from plan.md)

| # | change-id | goal | tasks | new deps | key risk (review focus) |
|---|---|---|---|---|---|
| 1 | `add-agent-governance-lint` | G1+G4 | 5 | none | false-negative = silent governance gap; agent-reachable misclass |
| 2 | `add-local-exchange-metric-strict-ratelimit` | G2 | 5 | none | `?`-restructure dropping a fail-closed outcome; strict-mode fail-open |
| 3 | `add-agent-tool-scope-sugar` | G3 | 5 | none | deny-wins violation; invalid sugar loading (must reject) |

## First dispatch

**`/kbd-apply add-agent-governance-lint`** — the governance-lint anchor.

## Standing constraints (in force)

Federate any JWKS IdM, Ory reference, never an IdP (load-bearing in the
JWKS-provider agent-reachable detection + the sugar-validates-to-Cedar). Blocking:
no secrets; admin port 4457 not public; no broken tests; config priority
CLI>env>YAML unchanged.
