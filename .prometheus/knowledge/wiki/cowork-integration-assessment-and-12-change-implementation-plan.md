---
type: Reference
id: cowork-integration-assessment-and-12-change-implementation-plan
title: cowork integration assessment and 12-change implementation plan
tags:
- cowork-cli
- prometheus-skill-pack
- skill-management
- install-pipeline
- plugin-management
- disk-space-guardian
- phase-plan
links:
- cowork-cli-integration-phase-goals-and-assessment-status
sources:
- stdin
- manual:cowork-integration
timestamp: 2026-07-03T21:31:09.530583+00:00
created_at: 2026-07-03T21:31:09.530583+00:00
updated_at: 2026-07-03T21:31:09.530583+00:00
revision: 0
---

## Context

- **Phase:** `cowork-integration`
- **Status:** `analyze_ready`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-03T21:29:44Z`
- **Source record:** `manual:cowork-integration`
- **Target codebase:** `git@github.com:GQAdonis/cowork-skills.git`

This assessment extends the phase goals captured in [cowork CLI integration phase goals and assessment status](/cowork-cli-integration-phase-goals-and-assessment-status.md). Three parallel research agents assessed the forked `cowork` Rust CLI, the `disk-space-guardian` project, and the existing `prometheus-skill-pack` install pipeline.

## Phase goals

- **G-01 — Architecture and integration plan:** Assess the `cowork` fork and define how to integrate it as a standard CLI in `prometheus-skill-pack`.
- **G-02 — New target platform support:** Add explicit `cowork` support for:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
- **G-03 — Skill-pack management awareness:** Make `cowork` able to update the pack, update toolchains, and repair broken installations.
- **G-04 — Plugin mechanics:** Make `cowork` understand Claude Code plugin and marketplace mechanics, and support installing/managing Codex and OpenCode plugins.
- **G-05 — Pipeline integration:** Integrate updated `cowork` into the skill-pack install pipeline and document it as the primary skill-management utility.

## Assessment findings

### cowork-skills fork

- The fork is a Rust CLI.
- It currently supports 16 agents.
- Missing target platforms must be added to `agents.rs`:
  - **Zed:** prometheus installs skills to `~/.config/zed/skills`; `cowork` has no support.
  - **Kimi Code CLI:** prometheus installs to `~/.kimi-code/skills` and wires `config.toml`; `cowork` has no support.
  - **MMX CLI:** prometheus detects the `mmx` binary; `cowork` has no support.
  - **Kimi Desktop:** skill directory path is still unknown and requires research.
  - **MiniMax Desktop:** skill directory path is still unknown and requires research.

### Structural gaps in cowork

- No MCP config wiring after skill installation.
  - prometheus already performs this wiring for Kimi, MiniMax, Codex, and OpenCode.
- No awareness of `prometheus-skill-pack` lifecycle operations:
  - pack status
  - pack update
  - pack repair
- No disk-space management.
  - No cleanup for Rust `target/` artifacts.
- `plugins install` only manages existing local plugins.
  - Missing git URL installation.
  - Missing manifest validation for plugin installs.

### disk-space-guardian

- `disk-space-guardian` is thoroughly specified but has **zero implementation code**.
- Five OpenSpec changes are defined and ready to execute.
- Planned `dsg` CLI directly addresses the disk growth problem:
  - `cargo build --release` for the prometheus substrate can create 1–4 GB of uncleaned artifacts per run.
- Recommended integration path:
  - expose disk cleanup through a delegating `cowork disk` subcommand.

### prometheus-skill-pack install pipeline

- Existing prometheus install scripts already support all 14 target platforms, including:
  - Zed
  - Kimi
  - MMX via binary detection
- The integration gaps are in `cowork`, not in prometheus install scripts.
- Direction: converge the tools so `cowork` becomes the user-facing interface over existing prometheus install logic.

## Proposed implementation plan

| Wave | Focus | Changes |
|---|---|---|
| 1 | Foundation and missing platforms | 001–003 |
| 2 | Plugin management for Codex, OpenCode, and Claude | 004–006 |
| 3 | Skill-pack awareness and disk-management stub | 007–009 |
| 4 | Pipeline integration and documentation | 010–012 |

## Open questions for analysis

1. What is the correct Kimi Desktop skill directory path?
2. What is the correct MiniMax Desktop skill directory path?
3. What is the MMX configuration format?
4. How should the `cowork` binary be distributed with `prometheus-skill-pack`?
5. Is `disk-space-guardian` scaffolding urgent for this integration, or should `cowork disk` start as a stub/delegation point?

## Current position

- **Last completed step:** `kbd-assess` for `cowork-integration`.
- **Current status:** `analyze_ready`.
- **Next action:** `/kbd-analyze cowork-integration`.

# Citations

1. stdin
2. manual:cowork-integration