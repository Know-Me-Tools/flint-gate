---
type: Reference
id: dsg-install-binaries-path-fix-phase-closes-with-v0-1-2-ci-follow-up
title: dsg install-binaries path fix phase closes with v0.1.2 CI follow-up
tags:
- disk-space-guardian
- dsg-cli
- install-pipeline
- github-actions
- release-workflow
- cargo-workspace
- prometheus-skill-pack
links:
- dsg-cli-foundation-phase-closed-with-v0-1-0-release-integration
sources:
- stdin
- manual:phase-dsg-install-binaries-fix
timestamp: 2026-07-04T19:34:42.150030+00:00
created_at: 2026-07-04T19:34:42.150030+00:00
updated_at: 2026-07-04T19:34:42.150030+00:00
revision: 0
---

## Context

- **Phase:** `phase-dsg-install-binaries-fix`
- **Status:** `reflect_complete` / closed
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/.claude/worktrees/charming-diffie-309eef`
- **Captured:** `2026-07-04T19:33:28Z`
- **Source record:** `manual:phase-dsg-install-binaries-fix`
- **Preceded by:** [dsg CLI foundation phase closed with v0.1.0 release integration](/dsg-cli-foundation-phase-closed-with-v0-1-0-release-integration.md)

This phase validated the `dsg` install path carried forward from the `phase-dsg-cli-foundation` work. The suspected issue was that `scripts/install-binaries.sh` might copy from a crate-subdirectory target path instead of the Cargo workspace target path.

## Goals and results

| Goal | Result | Notes |
|---|---:|---|
| **G-01** — Inspect `scripts/install-binaries.sh` `install_dsg()` target path | Met | Confirmed which Path A location the script uses. |
| **G-02** — Fix Path A if it used the wrong crate-subdir path | Met | No script change required; Path A was already correct. |
| **G-03** — Confirm `v0.1.0` release matrix and 4 release artifacts | Met with follow-up | `v0.1.0` had a CI path issue; fixes required `v0.1.1` and `v0.1.2`. |
| **G-04** — Run `bash scripts/install-binaries.sh` end-to-end | Met | `dsg` installed and verified on `PATH`. |

Final score: **4/4 goals met**.

## Install path finding

The correct Cargo build output for the `tools/disk-space-guardian/` workspace is:

```text
tools/disk-space-guardian/target/release/dsg
```

The incorrect crate-subdirectory path would be:

```text
tools/disk-space-guardian/dsg/target/release/dsg
```

`install_dsg()` in `scripts/install-binaries.sh` already used the correct workspace-root target path, so no install script patch was needed for Path A.

## Release workflow finding

The actual gap was not the local install script. The release CI workflow used a wrong `SRC` path with a `dsg/target/` prefix for the CI workspace build.

Key lesson:

```text
cargo build --manifest-path <crate>/Cargo.toml
```

places build output at:

```text
target/<triple>/release/
```

relative to the workspace root, not inside the crate subdirectory.

This affected GitHub Releases download behavior for Path B because release artifacts depend on CI packaging from the correct target directory.

## Release follow-up status

- `v0.1.0`: revealed the release workflow target-path issue.
- `v0.1.1`: part of the fix sequence.
- `v0.1.2`: reached **3/4 green** in the release matrix.
- `dsg`: installed and verified on `PATH` locally.

## Carry-forwards

- **CF-01:** macOS 13 artifact remains outstanding.
- **CF-02:** install script submodule guard fix remains outstanding; background task spawned.

# Citations

1. stdin
2. manual:phase-dsg-install-binaries-fix