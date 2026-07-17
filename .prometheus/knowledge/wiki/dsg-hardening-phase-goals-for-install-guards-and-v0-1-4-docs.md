---
type: Reference
id: dsg-hardening-phase-goals-for-install-guards-and-v0-1-4-docs
title: dsg hardening phase goals for install guards and v0.1.4 docs
tags:
- disk-space-guardian
- dsg-cli
- install-pipeline
- git-submodules
- release-management
- prometheus-skill-pack
- kbd-phase
links:
- dsg-install-binaries-path-fix-phase-closes-with-v0-1-2-ci-follow-up
- cowork-submodule-integration-at-v0-1-5-before-final-validation
- cowork-push-and-release-phase-goals-for-v0-2-0-submodule-update
sources:
- stdin
- manual:phase-dsg-hardening
timestamp: 2026-07-05T18:33:08.945867+00:00
created_at: 2026-07-05T18:33:08.945867+00:00
updated_at: 2026-07-05T18:33:08.945867+00:00
revision: 0
---

## Context

`phase-dsg-hardening` was opened to close carry-forwards from [dsg install-binaries path fix phase closes with v0.1.2 CI follow-up](/dsg-install-binaries-path-fix-phase-closes-with-v0-1-2-ci-follow-up.md).

Carry-forwards:

- **CF-02 — submodule guard bug:** `scripts/install-binaries.sh` used guards like `if [ -d "${dir}" ]`, which pass when a submodule directory exists but is uninitialized. Under `set -euo pipefail`, attempting `cargo build` in such a directory fails because there is no `Cargo.toml`, aborting the whole script before later tool sections such as `dsg` can run.
- **CF-03 — broken Path B release assets:** `dsg` releases `v0.1.0` and `v0.1.1` have broken or unusable Path B assets. Skill documentation must direct users to `v0.1.4+`, and the Path B download flow must be validated against the corrected release.

Additional state:

- `tools/disk-space-guardian` in `prometheus-skill-pack` remains pinned to the commit that became `v0.1.3`.
- `v0.1.4` required no `dsg` code change; only the runner/release workflow changed.
- `v0.1.4` CI is now green: all 4 jobs completed and all 4 artifacts were uploaded.
- The `macos-latest` fix completed the previously stuck `x86_64-apple-darwin` job in 51 seconds.

## Phase metadata

- **Phase:** `phase-dsg-hardening`
- **Project:** `unspecified`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-05T18:12:47Z`
- **Stage:** `assess`
- **Next command:** `/kbd-assess phase-dsg-hardening`

## Goals

| Goal | Description |
|---|---|
| **G-01** | Fix `install-binaries.sh` submodule guards by replacing directory-existence checks with `Cargo.toml` presence checks for all tool sections. |
| **G-02** | Update `skills/devops/disk-space-guardian/SKILL.md` to reference `v0.1.4` as the recommended install version and confirm Path B download flow. |
| **G-03** | Advance `tools/disk-space-guardian` submodule pointer to `v0.1.4`. |
| **G-04** | Verify a full `install-binaries.sh` end-to-end run completes without aborting when submodules are uninitialized. |

## Required implementation detail

Use file-based submodule guards instead of directory-based guards:

```bash
# Bad: passes for uninitialized submodule directories
if [ -d "${dir}" ]; then
  cargo build --release
fi

# Good: only builds when the Rust crate/workspace is present
if [ -f "${dir}/Cargo.toml" ]; then
  cargo build --release
fi
```

Apply the guard pattern consistently across tool sections, including:

- `pk`
- `cowork`
- `dsg`
- any other `install-binaries.sh` sections that build from submodule directories

This hardening aligns with previous submodule/install-pipeline issues such as [cowork submodule integration at v0.1.5 before final validation](/cowork-submodule-integration-at-v0-1-5-before-final-validation.md) and the later [cowork push-and-release phase goals for v0.2.0 submodule update](/cowork-push-and-release-phase-goals-for-v0-2-0-submodule-update.md).

## Release documentation decision

- Recommended `dsg` version for skill documentation: **`v0.1.4` or newer**.
- Avoid pointing users at `v0.1.0` or `v0.1.1` for Path B installs because their release assets are broken/unusable.
- `v0.1.4` is the first confirmed release state in this phase context with all 4 CI jobs green and all 4 artifacts uploaded.

# Citations

1. stdin
2. manual:phase-dsg-hardening