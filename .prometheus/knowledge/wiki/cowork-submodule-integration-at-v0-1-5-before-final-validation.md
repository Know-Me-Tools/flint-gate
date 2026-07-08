---
type: Reference
id: cowork-submodule-integration-at-v0-1-5-before-final-validation
title: cowork submodule integration at v0.1.5 before final validation
tags:
- cowork-cli
- git-submodules
- prometheus-skill-pack
- install-pipeline
- validation
- rust-cli
links:
- cowork-cli-integration-phase-goals-and-assessment-status
- cowork-integration-assessment-and-12-change-implementation-plan
- cowork-cli-ci-and-release-workflow-integration
sources:
- stdin
- manual:cowork-integration
timestamp: 2026-07-04T14:55:25.740277+00:00
created_at: 2026-07-04T14:55:25.740277+00:00
updated_at: 2026-07-04T14:55:25.740277+00:00
revision: 0
---

## Context

- **Phase:** `cowork-integration`
- **Stage:** `execute`
- **Step:** `23 of 24`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-04T14:54:03Z`
- **Target codebase:** `git@github.com:GQAdonis/cowork-skills.git`
- **Completed change:** `change-int-006`
- **Commit:** `bc40a90`
- **Submodule revision:** `tools/cowork-skills` at `53e6b31` (`v0.1.5`)

This checkpoint continues the earlier [cowork CLI integration phase goals and assessment status](/cowork-cli-integration-phase-goals-and-assessment-status.md), the [cowork integration assessment and 12-change implementation plan](/cowork-integration-assessment-and-12-change-implementation-plan.md), and the [cowork CLI CI and release workflow integration](/cowork-cli-ci-and-release-workflow-integration.md).

## Completed Work

`change-int-006` was completed, verified, archived, and committed at `bc40a90`.

Key result:

- `tools/cowork-skills` is now a proper Git submodule/gitlink.
- The submodule points to `cowork-skills` revision `53e6b31`, tagged `v0.1.5`.
- `install-binaries.sh` now has `install_cowork()` build both binaries from `cli/`:
  - `cowork`
  - `co`

## Phase Goals in Scope

The phase remains focused on integrating the forked `cowork` CLI as the standard `prometheus-skill-pack` installation and management utility:

- **G-01:** Assess the forked codebase and define the architecture/integration plan.
- **G-02:** Add explicit support for:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
- **G-03:** Make `cowork` aware of `prometheus-skill-pack` management operations:
  - pack updates
  - toolchain updates
  - installation repair
- **G-04:** Model Claude Code plugin and marketplace mechanics and extend support to Codex and OpenCode plugins.
- **G-05:** Integrate the updated CLI into the skill-pack install pipeline and document it as the primary skill-management utility.

## Remaining Work

Only one change remains:

```text
/kbd-apply change-int-007
```

`change-int-007` is the final validation/integration pass and must include:

- Run full `npm run validate:strict`.
- Update CI workflow for submodule checkout.
- Run `npm run build` to rebuild marketplace symlinks.
- Perform smoke-test verification.

## Current Position

```text
Phase:    cowork-integration
Step:     23 of 24
Stage:    execute
Next cmd: /kbd-apply change-int-007
Done:     change-int-006 (tools/cowork-skills submodule at 53e6b31 v0.1.5)
Remaining: change-int-007 (final validate + CI update + npm build) — LAST CHANGE
```

# Citations

1. stdin
2. manual:cowork-integration