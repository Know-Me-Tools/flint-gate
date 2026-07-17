# add-changelog

**Phase:** beta-release-readiness / Phase 3 (Medium gap M-1)

## Problem

There is no `CHANGELOG.md` at the repository root. Beta customers and
operators need to know what changed between versions so they can assess
upgrade risk. The `release.yml` workflow also does not verify that a
changelog entry exists before publishing a release, so new releases can
go out with no documented changes.

## Solution

1. Create `CHANGELOG.md` at the repo root following Keep a Changelog
   format (https://keepachangelog.com/en/1.1.0/)
2. Add an initial entry capturing all pre-beta work: Cedar authorization,
   approval flows, streaming, admin UI, Go/TS SDKs
3. Add a `check-changelog` step to `.github/workflows/release.yml` that
   fails if the Unreleased section is empty when a release is triggered

The `check-changelog` step uses `grep` to confirm `## [Unreleased]` section
has content beyond the section header — this is a lightweight guard, not a
full validation tool.

## Files to change

- `CHANGELOG.md` (new) — root-level changelog with initial entry
- `.github/workflows/release.yml` — add `check-changelog` step before
  the build/publish steps
