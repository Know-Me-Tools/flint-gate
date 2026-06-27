# Execution — production-readiness

**Phase:** production-readiness
**Executed:** 2026-06-23
**Backend:** OpenSpec (`openspec/changes/`)
**Plan:** `plan.md` (8 ordered changes)

---

## Backend selection

OpenSpec is available (`openspec/` directory) and all 8 changes already have `proposal.md` + `tasks.md` under `openspec/changes/`. Backend: **openspec**.

## Dispatch contract

- Each change is applied in plan order (#1 → #8).
- After each change: `cargo test --workspace && cargo clippy --workspace -- -D warnings` must pass.
- QA gate (artifact-refiner): **skipped** for changes with <3 files modified or docs-only changes (#1, #8 K8s YAML). Applied to substantive code changes.
- On pass: mark change `done` in `progress.json`, archive OpenSpec change.
- On fail: mark change `blocked`, halt.

## Execution order

| Step | Change | Status |
|---|---|---|
| 1 | `fix-k8s-readiness-probe` | pending |
| 2 | `implement-route-host-filter` | pending |
| 3 | `wire-stream-metadata-injection` | pending |
| 4 | `implement-jwt-key-rotation` | pending |
| 5 | `implement-redis-l2-cache` | pending |
| 6 | `extract-stream-processor-trait` | pending |
| 7 | `implement-ndjson-streaming` | pending |
| 8 | `implement-session-watchdog` | pending |
