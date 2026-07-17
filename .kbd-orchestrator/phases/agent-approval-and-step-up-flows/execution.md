# Execution — agent-approval-and-step-up-flows

_Execute stage 2026-07-08. Backend selected + dispatch contract; task-walking is
deferred to `/kbd-apply` (one task/turn, KBD-owned loop)._

## Backend selection

| Field | Value |
|-------|-------|
| Backend | **openspec** (`kbd-apply detect` → openspec) |
| Driver | **`/kbd-apply`** (never bare `/opsx:apply`) |
| Changes | 3, dependency-ordered G3→G2→G1 (from `plan.md`; inverts the seed) |
| Tasks | 6 / 6 / 3 per change (from each `tasks.md`) |

## Dispatch contract (per change, one task/turn)

For each change in `plan.md` order:

```
kbd-apply list <change>            # TSV: id  done  title
kbd-apply begin-task <change> …    # fires task:before + "Starting task i of n"
   <implement EXACTLY that one task: edit code/docs>
kbd-apply end-task <change> …      # marks done, syncs progress.json + waypoint,
                                   # fires task:after + "Completed task i of n"
```

After the LAST task of a change:
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<id>/refinement_log.md`.
2. **separated security-reviewer** — changes 1 & 2 (approval/stream/authz seam;
   author never grades its own). **`add-approval-timeout-and-janitor` requires an
   explicit "no silent-allow, no half-open stream on timeout" check** (highest-
   priority security item this phase). `fix-approval-flow-comments-and-verify` is
   verify-only → a code-reviewer pass suffices.
3. The `specs/approval/` delta (already written per change) makes `verify` pass on
   merit.
4. `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` green;
   ≥80% new-code coverage; **web build + typecheck green for change 2**.
5. `kbd-apply verify <change>` → `openspec archive <change> --skip-specs --yes`.
6. Sync `progress.json` (status archived, `changes_completed`++) + advance the
   waypoint to the next change **in SEPARATE commands** (pipeline-enforce hook).

## Change order + gate emphasis

1. **`add-approval-timeout-and-janitor`** (G3, safety-critical) — the `sleep_until`
   auto-deny arm MUST emit the deny event + resume to termination (no silent-allow,
   no half-open stream); `enabled:false` MUST deny. Fail-closed tests: timeout→deny,
   staggered deadlines, enabled:false→deny, janitor reaps.
2. **`add-pending-approvals-surface`** (G2, depends on 1) — `list()` skips expired;
   GET endpoints admin-router only; web tab with poll. Fail-safe test: list skips
   expired + admin-only.
3. **`fix-approval-flow-comments-and-verify`** (G1, last) — comment fix + an
   end-to-end pause/approve/deny test asserting no silent-allow path.

## First dispatch

`/kbd-apply add-approval-timeout-and-janitor`
