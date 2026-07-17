---
type: Reference
id: cowork-push-and-release-phase-goals-for-v0-2-0-submodule-update
title: cowork push-and-release phase goals for v0.2.0 submodule update
tags:
- cowork-cli
- prometheus-skill-pack
- git-submodules
- release-management
- install-pipeline
- kbd-phase
links:
- cowork-integration-phase-closed-with-submodule-release-blocker
- cowork-cli-integration-phase-goals-and-assessment-status
- cowork-integration-assessment-and-12-change-implementation-plan
- cowork-cli-ci-and-release-workflow-integration
sources:
- stdin
- manual:phase-cowork-push-and-release
timestamp: 2026-07-04T15:18:29.707108+00:00
created_at: 2026-07-04T15:18:29.707108+00:00
updated_at: 2026-07-04T15:18:29.707108+00:00
revision: 0
---

## Context

The `cowork-integration` phase delivered 10 Rust commits into the local `cowork-skills` worktree, but the commits were not yet pushed to the fork remote. This left `prometheus-skill-pack` installing the unextended upstream `cowork` binary.

- **Phase:** `phase-cowork-push-and-release`
- **Stage:** `assess`
- **Created at commit:** `5af38fb`
- **Phase directory:** `.kbd-orchestrator/phases/phase-cowork-push-and-release/`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Local cowork worktree:** `/Users/gqadonis/Projects/prometheus/cowork-skills`
- **Target remote:** `git@github.com:GQAdonis/cowork-skills.git`
- **Captured:** `2026-07-04T15:15:35Z`

This phase directly resolves the release blocker recorded when [cowork-integration closed with an unadvanced submodule](/cowork-integration-phase-closed-with-submodule-release-blocker.md). The pending commits include the integration work described across the earlier [cowork CLI integration goals](/cowork-cli-integration-phase-goals-and-assessment-status.md), [12-change implementation plan](/cowork-integration-assessment-and-12-change-implementation-plan.md), and [CI/release workflow integration](/cowork-cli-ci-and-release-workflow-integration.md).

## Problem

`tools/cowork-skills` in `prometheus-skill-pack` still points at upstream tag `v0.1.5` commit `53e6b31`.

Impact:

- `install_cowork()` in `install-binaries.sh` builds the old, unextended binary.
- Installed `cowork` lacks the integration-phase support for:
  - Zed
  - Kimi Code CLI
  - MMX CLI
  - Kimi Desktop
  - MiniMax Desktop
  - pack management
  - toolchain management
  - disk support

## Phase goals

| Goal | Required outcome |
|---|---|
| **G-01** | Push the 10 local `cowork-integration` commits from `/Users/gqadonis/Projects/prometheus/cowork-skills` to `git@github.com:GQAdonis/cowork-skills.git` on `main`. |
| **G-02** | Tag semver release `v0.2.0` on the `cowork-skills` remote and confirm the CI workflow passes. |
| **G-03** | Advance the `tools/cowork-skills` submodule pointer in `prometheus-skill-pack` to the new `cowork-skills` HEAD, commit the pointer update, and verify `git submodule status` is clean. |
| **G-04** | Confirm the installed binary works end-to-end on this machine for `cowork pack status` and `cowork toolchain status`. |

## Expected execution sequence

```text
assess → plan → execute
```

Execution work should cover:

1. Push local `cowork-skills` commits to fork `main`.
2. Create and push release tag `v0.2.0`.
3. Verify GitHub Actions / CI passes for the release.
4. Update `prometheus-skill-pack/tools/cowork-skills` to the new commit.
5. Commit the submodule pointer update in `prometheus-skill-pack`.
6. Verify submodule cleanliness with `git submodule status`.
7. Reinstall or otherwise validate the resulting binary.
8. Smoke test:

```bash
cowork pack status
cowork toolchain status
```

## Position snapshot

```text
Phase:    phase-cowork-push-and-release
Step:     0 of 0
Stage:    assess
Next cmd: /kbd-assess phase-cowork-push-and-release
Remaining: assess → plan → execute (push commits, tag v0.2.0, advance submodule pointer)
```

# Citations

1. stdin
2. manual:phase-cowork-push-and-release