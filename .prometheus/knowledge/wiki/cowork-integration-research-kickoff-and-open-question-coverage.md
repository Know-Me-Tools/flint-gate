---
type: Reference
id: cowork-integration-research-kickoff-and-open-question-coverage
title: cowork integration research kickoff and open-question coverage
tags:
- cowork-cli
- prometheus-skill-pack
- skill-management
- install-pipeline
- plugin-management
- research-status
- phase-plan
links:
- cowork-cli-integration-phase-goals-and-assessment-status
- cowork-integration-assessment-and-12-change-implementation-plan
sources:
- stdin
- manual:cowork-integration
timestamp: 2026-07-03T21:34:27.264303+00:00
created_at: 2026-07-03T21:34:27.264303+00:00
updated_at: 2026-07-03T21:34:27.264303+00:00
revision: 0
---

## Context

- **Phase:** `cowork-integration`
- **Project:** `unspecified`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-03T21:32:38Z`
- **Source record:** `manual:cowork-integration`
- **Target codebase:** `git@github.com:GQAdonis/cowork-skills.git`

The phase investigates integration of the forked `cowork` CLI into `prometheus-skill-pack` as the standard installation and management CLI. Investigation work is to occur in a dedicated worktree outside the skill-pack directory to avoid polluting the main tree.

This checkpoint continues the earlier [cowork CLI integration phase goals and assessment status](/cowork-cli-integration-phase-goals-and-assessment-status.md) and follows the broader implementation planning in [cowork integration assessment and 12-change implementation plan](/cowork-integration-assessment-and-12-change-implementation-plan.md).

## Phase goals

- **G-01 — Architecture and integration plan:** Investigate the `cowork` fork and produce an architecture assessment with a clear integration plan for adding it as a standard CLI in the prometheus skill pack.
- **G-02 — New target platform support:** Add explicit `cowork` support for:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
- **G-03 — Skill-pack management awareness:** Make `cowork` aware of how `prometheus-skill-pack` is managed so it can:
  - update the pack
  - update toolchains
  - repair broken installations
- **G-04 — Plugin mechanics:** Make `cowork` understand Claude Code plugin and marketplace mechanics in full detail, and update it to support installing and managing:
  - Codex plugins
  - OpenCode plugins
- **G-05 — Install pipeline integration:** Integrate the updated `cowork` CLI into the skill-pack install pipeline and document its usage as the primary skill-management utility.

## Research status

- **Status:** `analyze_ready`
- **Current activity:** Four parallel Tier 1–3 research agents were launched to cover all open questions.
- **Pending next step:** Run `/kbd-analyze cowork-integration` after agent completion to synthesize results.

## Active research tracks

The four parallel research agents are covering:

1. **Kimi and MiniMax Desktop paths**
   - Locate expected skill/plugin installation paths.
   - Determine desktop-specific packaging or config requirements.
2. **MMX and Zed format support**
   - Determine target layout and metadata formats for MMX CLI and Zed.
   - Identify how skills should be installed, updated, and discovered.
3. **Binary distribution**
   - Determine how the updated `cowork` CLI should be built, distributed, and wired into the skill-pack installer.
   - Assess cross-platform binary packaging implications.
4. **OpenCode and Codex plugin formats**
   - Research plugin structure and installation mechanics.
   - Map plugin management requirements into `cowork` operations.

## Position marker

```text
Position: cowork-integration | status: analyze_ready
Last: Four parallel Tier 1-3 research agents launched — Kimi/MiniMax Desktop paths, MMX/Zed format, binary distribution, OpenCode/Codex plugin formats.
Next: /kbd-analyze cowork-integration (synthesis pending agent completion)
```

# Citations

1. stdin
2. manual:cowork-integration