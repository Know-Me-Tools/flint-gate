---
type: Reference
id: cowork-cli-ci-and-release-workflow-integration
title: cowork CLI CI and release workflow integration
tags:
- cowork-cli
- github-actions
- release-automation
- prometheus-skill-pack
- install-pipeline
- rust-cli
links:
- cowork-integration-assessment-and-12-change-implementation-plan
- cowork-cli-integration-phase-goals-and-assessment-status
- cowork-integration-research-kickoff-and-open-question-coverage
sources:
- stdin
- manual:cowork-integration
timestamp: 2026-07-04T13:02:42.540019+00:00
created_at: 2026-07-04T13:02:42.540019+00:00
updated_at: 2026-07-04T13:02:42.540019+00:00
revision: 0
---

## Context

- **Phase:** `cowork-integration`
- **Stage:** `execute`
- **Step:** `15 of 24`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-04T12:51:40Z`
- **Target codebase:** `git@github.com:GQAdonis/cowork-skills.git`
- **Completed change:** `change-cowork-010`
- **Commit:** `f0f695a`

This checkpoint is part of the broader [cowork integration assessment and 12-change implementation plan](/cowork-integration-assessment-and-12-change-implementation-plan.md) and continues the [cowork CLI integration phase goals and assessment status](/cowork-cli-integration-phase-goals-and-assessment-status.md).

## Phase goals

- **G-01:** Assess the forked `cowork` codebase and produce an architecture/integration plan for adding it as a standard CLI in `prometheus-skill-pack`.
- **G-02:** Add explicit support for target platforms:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
- **G-03:** Make `cowork` aware of `prometheus-skill-pack` management so it can update the pack, update toolchains, and repair broken installations.
- **G-04:** Model Claude Code plugin and marketplace mechanics in detail, and support Codex/OpenCode plugin installation and management.
- **G-05:** Integrate the updated `cowork` CLI into the skill-pack install pipeline and document it as the primary skill-management utility.

## Completed change: `change-cowork-010`

`change-cowork-010` added CI and release automation to the forked `cowork-skills` repository.

### CI workflow

Added `.github/workflows/ci.yml` in `cowork-skills`:

- Runs on push and pull request.
- Executes:
  - `cargo fmt`
  - `cargo clippy`
  - `cargo test`
- Uses an Ubuntu + macOS matrix.

### Release workflow

Added `.github/workflows/release.yml` in `cowork-skills`:

- Builds a four-target release matrix:
  - `x86_64-linux-musl`
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-windows-msvc`
- Uploads release archives to GitHub Releases via `softprops/action-gh-release@v2`.
- Artifact naming convention:
  - `cowork-{version}-{target}.tar.gz`
  - `cowork-{version}-{target}.zip`
- Includes optional crates.io publish support.

### Cargo distribution metadata

Updated `cli/Cargo.toml` with `[package.metadata.dist]` covering all four release targets.

## Next planned change

Next command:

```text
/kbd-apply change-cowork-011
```

Planned work for `change-cowork-011`:

- Add `install_cowork()` to `install-binaries.sh`.
- Wire `cowork-skills` as a git submodule at:

```text
tools/cowork-skills
```

This next step begins integrating the release-capable `cowork` binary into the `prometheus-skill-pack` install pipeline, aligning with the installation-management direction described in [cowork integration research kickoff and open-question coverage](/cowork-integration-research-kickoff-and-open-question-coverage.md).

# Citations

1. stdin
2. manual:cowork-integration