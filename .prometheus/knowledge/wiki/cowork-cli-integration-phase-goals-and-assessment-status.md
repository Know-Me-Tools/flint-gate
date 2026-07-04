---
type: Reference
id: cowork-cli-integration-phase-goals-and-assessment-status
title: cowork CLI integration phase goals and assessment status
tags:
- cowork-cli
- prometheus-skill-pack
- skill-management
- install-pipeline
- plugin-management
- phase-plan
sources:
- stdin
- manual:cowork-integration
- /Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef
timestamp: 2026-07-03T21:28:21.901308+00:00
created_at: 2026-07-03T21:28:21.901308+00:00
updated_at: 2026-07-03T21:28:21.901308+00:00
revision: 0
---

## Context

- **Phase:** `cowork-integration`
- **Project:** `unspecified`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-03T21:23:06Z`
- **Source record:** `manual:cowork-integration`
- **Target codebase:** `git@github.com:GQAdonis/cowork-skills.git`

The phase investigates integrating the forked `cowork` CLI utility into `prometheus-skill-pack` as the standard installation and management CLI. Work is intended to occur in a dedicated worktree outside the skill-pack directory to avoid polluting the main tree during investigation.

## Goals

- **G-01 — Architecture and integration plan:** Investigate the `cowork` fork and produce an architecture assessment with a clear integration plan for adding it as a standard CLI in the prometheus skill pack.
- **G-02 — New target platform support:** Add explicit support for:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
- **G-03 — Skill-pack management awareness:** Make `cowork` aware of how `prometheus-skill-pack` is managed so it can:
  - update the pack
  - update toolchains
  - repair broken installations
- **G-04 — Plugin and marketplace mechanics:** Make `cowork` fully understand Claude Code plugin and marketplace mechanics, and update it to support installing and managing:
  - Codex plugins
  - OpenCode plugins
- **G-05 — Install pipeline integration:** Integrate the updated `cowork` CLI into the skill-pack install pipeline and document it as the primary skill-management utility.

## Current status

- **Position:** `cowork-integration`
- **Status:** `assessment_ready`
- **Last completed action:** Three parallel research agents were launched:
  - `cowork` fork investigation
  - `disk-space-guardian` investigation
  - skill-pack install pipeline investigation
- **Next action:** Run `/kbd-assess cowork-integration` after agent completion to synthesize findings.

# Citations

1. stdin
2. manual:cowork-integration
3. /Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef