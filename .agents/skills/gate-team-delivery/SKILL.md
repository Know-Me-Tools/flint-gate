---
name: gate-team-delivery
description: "Coordinate bounded Flint Gate changes, reconcile KBD state, check Compass freshness and produce cross-harness handoffs."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-team-delivery

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Read the assigned task, project instructions and docs/agent-teams/flint-gate/WORKFLOW.md. The roster is a specialist pool, not permission to edit the whole repository or run all roles.

1. Inspect working-tree status and reserve exact write paths. Other writers are active; preserve their changes. Route overlapping work to the current owner before editing.
2. Reconcile canonical KBD state for this task: project.json, waypoint and phase progress currently disagree. Do not resume a historical waypoint or hand-edit generated projections. Use the selected project's KBD/OpenSpec workflow for application changes.
3. Use compass-flint-gate MCP or Compass CLI to inspect affected callers. Run node .compass/verify.mjs to check digests; refresh a stale graph before trusting it. SQL, macros, dynamic dispatch and network consumers require source/contract inspection.
4. Choose the smallest team: one implementer for isolated work, architect plus implementer plus independent reviewer for identity or policy changes. Prefer at most four active writers and one Rust build owner. These are coordination rules, not native enforcement.
5. Produce acceptance criteria and a dependency DAG for the selected task. Run focused tests that prove changed behavior and regression boundaries; do not claim a whole-workspace build from structural indexing.
6. Handoff with repo/branch/HEAD, dirty paths, owned paths, acceptance evidence, graph digest, exact tests/results and unresolved risks. Use agent-team-handoff when installed, otherwise preserve the same fields locally. Never put tokens, signing keys or customer data into logs/shared memory.

Gate authenticates/enforces at the edge; Forge owns database/RLS semantics; Fabric owns event transport; UAR or BossFang owns the selected execution loop; the-boss is the controller; KnowMe consumes identity and app services. These are intended contracts, not proof of integration. Read current counterpart contracts and send findings to their owners; this team writes only Gate.

