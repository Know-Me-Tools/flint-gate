# Execution — agent-gateway-exposure-operability

_Backend: **openspec** · Driver: **/kbd-apply** (one task per turn) · 4 changes._

## Backend selection

OpenSpec is present (`openspec/changes/` holds the 4 proposals authored in Spec)
and the phase was seeded `backend: openspec`. Spec-backed traceability is
required (this phase enforces security invariants). **Backend = openspec.**

> Task execution is driven by **`/kbd-apply`** — never bare `/opsx:apply` (which
> is KBD-unaware: no hooks, no progress.json, no waypoint). `/kbd-execute` only
> writes this dispatch contract.

## Dispatch contract

For each change, in order:

1. `/kbd-apply <change-id>` walks its `tasks.md` one task per turn
   (`begin-task` → implement → `end-task`), syncing `progress.json` + waypoint
   and firing per-task hooks.
2. After the final task, the **QA gate**: artifact-refiner constraint validation
   (`.kbd-orchestrator/constraints.md`) + a **separated security review**
   (security-reviewer agent — the author never grades its own fail-open seam)
   → `.refiner/artifacts/<change-id>/refinement_log.md`.
3. On PASS: `openspec archive <change-id> --skip-specs --yes` (phase convention —
   no `specs/` capability delta). On FAIL: mark BLOCKED, refine, re-gate.
4. Sync `progress.json` (`changes_completed++`) then advance the waypoint —
   **in separate commands** (the pipeline-enforce hook reads the pre-update count
   if combined).

## Per-change verification gate (all four)

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; **every new auth/exposure path has a fail-closed
(deny / refuse-start) test**.

## Ordered changes (from plan.md)

| # | change-id | goal | tasks | new deps | key risk (review focus) |
|---|---|---|---|---|---|
| 1 | `add-oauth-shared-ratelimit` | G1 | 5 | none (reuse RedisRateLimiter) | rate-limit bypass; outage-posture fail-open |
| 2 | `add-exposure-guardrails` | G3 | 5 | none (mirror admin_auth_posture) | refuse-start fail-open; http:// slipping through |
| 3 | `add-delegate-observability` | G2 | 5 | metrics@0.24, metrics-exporter-prometheus@0.18 | /metrics leaking to proxy port; token bytes in labels |
| 4 | `add-oauth-e2e-ory` | G4 | 4 | none (existing Playwright) | flaky waits; real-stack orchestration |

## First dispatch

**`/kbd-apply add-oauth-shared-ratelimit`** — the horizontal-exposure gate.

## Standing constraints (in force)

Federate any JWKS IdM, Ory reference, never an IdP. Blocking: no secrets
committed; admin port 4457 not public (change 3 `/metrics` = admin-port,
compliant); no broken tests; config priority CLI>env>YAML unchanged.
