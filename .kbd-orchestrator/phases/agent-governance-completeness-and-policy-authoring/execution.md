# Execution — agent-governance-completeness-and-policy-authoring

_Execute stage 2026-07-07. Backend selected + dispatch contract; task-walking is
deferred to `/kbd-apply` (one task/turn, KBD-owned loop)._

## Backend selection

| Field | Value |
|-------|-------|
| Backend | **openspec** (`kbd-apply detect` → openspec; `openspec/` present) |
| Driver | **`/kbd-apply`** (never bare `/opsx:apply` — no KBD hooks/progress/waypoint) |
| Changes | 3, dependency-ordered G1→G2→G3 (from `plan.md`) |
| Tasks | 5 per change (from each `tasks.md`) |

## Dispatch contract (per change, one task/turn)

For each change in `plan.md` order:

```
kbd-apply list <change>            # TSV: id  done  title
kbd-apply begin-task <change> …    # fires task:before + "Starting task i of n"
   <implement EXACTLY that one task: edit code/docs>
kbd-apply end-task <change> …      # marks done, syncs progress.json + waypoint,
                                   # fires task:after + "Completed task i of n"
```

After the LAST task of a change (5/5):
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<id>/refinement_log.md`.
2. **separated security-reviewer** — full review on every change (authz/policy/
   reload seam; author never grades its own). **`admin-tool-scope-builder`
   additionally requires an explicit Cedar string-concatenation injection re-check
   at the admin-API boundary** (highest-priority security item this phase).
3. **Add the `specs/` delta already written** (each change has one) so
   `openspec validate --strict` / `kbd-apply verify` passes on merit.
4. `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` green;
   ≥80% new-code coverage; **web build green for `admin-tool-scope-builder`**.
5. `kbd-apply verify <change>` → `openspec archive <change> --skip-specs --yes`.
6. Sync `progress.json` (change status archived, `changes_completed`++) and
   advance the waypoint to the next change **in SEPARATE commands**
   (pipeline-enforce hook blocks a reflect/next reference while incomplete).

## Change order + gate emphasis

1. **`lint-db-sourced-routes`** (G1) — reload error model: WARN always; strict →
   reject-route + retain-last-good (never terminate). Fail-closed test: reload
   strict retains last-good.
2. **`merge-agent-tool-policies-into-engine`** (G2) — overlay on `AuthzEngine`,
   concatenate on every reload, remove the refuse-start guard. Fail-closed test:
   sugar survives a reload + deny-wins conflict matrix.
3. **`admin-tool-scope-builder`** (G3, depends on 2) — structured-only endpoint
   through `compile_and_validate`; no raw-Cedar bypass. Fail-closed test:
   illegal agent/tool → 400. Security-review injection re-check REQUIRED.

## First dispatch

`/kbd-apply lint-db-sourced-routes`
