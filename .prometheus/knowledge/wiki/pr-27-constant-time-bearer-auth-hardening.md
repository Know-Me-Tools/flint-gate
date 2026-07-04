---
type: Reference
id: pr-27-constant-time-bearer-auth-hardening
title: PR 27 constant-time bearer auth hardening
tags:
- bearer-auth
- constant-time
- security-review
- ci-validation
- forge-rs
- cross-model-qa
sources:
- stdin
- manual:phase-ci-cross-model-qa-and-hardening
timestamp: 2026-07-03T20:03:29.013165+00:00
created_at: 2026-07-03T20:03:29.013165+00:00
updated_at: 2026-07-03T20:03:29.013165+00:00
revision: 0
---

## Session Outcome

- **Phase:** `phase-ci-cross-model-qa-and-hardening`
- **Project root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack`
- **Captured:** `2026-07-03T20:01:46Z`
- **Status:** `execute_complete`
- **Progress:** 3/3 changes complete; 3/3 goals met
- **PR:** #27
- **Merge state:** `CLEAN`
- **CI:** all 9 `validate.yml` jobs green; 0 failing
- **Commit:** `bbef4e9` after rebase to a single auth commit

## Implemented Change: Constant-Time Bearer Auth

Replaced deprecated bearer validation with a custom constant-time validator:

- Removed deprecated `ValidateRequestHeaderLayer::bearer` usage.
- Added custom `BearerAuth` implementing `ValidateRequest`.
- Used `subtle::ConstantTimeEq` for token comparison.
- Removed `#[allow(deprecated)]`.
- Removed `TODO(security)`.

## Security Review Findings and Fixes

The initial implementation was routed through security review before shipping. Findings were fixed before CI completion.

### MEDIUM: Empty or whitespace token auth bypass

**Issue:** If `FORGE_MCP_TOKEN` was set but empty or whitespace, the unset-token fallback was skipped. This allowed an empty bearer token such as `Bearer ` to authenticate.

**Fix:** Blank or whitespace-only `FORGE_MCP_TOKEN` is now treated as absent, causing a random token to be minted instead.

**Regression coverage:** Added test for blank-token handling.

### HIGH: Incorrect constant-time behavior documentation

**Issue:** A code comment falsely claimed `subtle` does not early-out on length.

**Fix:** Comment corrected. `subtle` can reveal non-secret token length, but closes per-byte short-circuit timing leakage.

### LOW: Infallible 401 construction

**Issue:** Initial 401 response construction used `expect()`.

**Fix:** 401 response is now built infallibly, satisfying the forge-rs constitution constraint against avoidable panics.

## Verification

### Unit Tests

Added 6 `BearerAuth` unit tests covering:

- Accept valid token
- Reject wrong token
- Reject missing token
- Reject non-`Bearer` authorization scheme
- Reject invalid bearer prefix
- Reject empty token

### End-to-End Service Checks

Validated against the live service on `:8943`:

- `GET /health` without auth returns `200`
- `POST /mcp` without token returns `401`
- `POST /mcp` with wrong token returns `401`
- `POST /mcp` with correct token returns `200`

### CI

PR #27 passed all 9 jobs in `validate.yml`, including:

- `forge-rs-test`
- BDD suite
- Check Rust CLI
- Remaining validation jobs

## Process Notes

- PR #27 was rebased onto `main` after PR #26 merged.
- Diff was reduced to the single auth commit `bbef4e9`.
- PR was retargeted to `main` because `validate.yml` only triggers on `main`.
- Used `git commit -F` to avoid shell expansion problems with GitHub Actions syntax like `${{ }}`.

## Phase Completion State

Completed changes:

1. Toolchain change merged.
2. Cross-model QA change merged.
3. Constant-time bearer auth completed on PR #27 with CI and security review passing.

## Remaining Follow-Up

- Merge PR #27.
- Run `/kbd-reflect phase-ci-cross-model-qa-and-hardening` to close the phase.
- Provision `ANTHROPIC_API_KEY` for **OQ-A3** so cross-model QA can execute real review dispatches. The workflow now loads cleanly and no longer shows red, but cannot perform actual review runs without the secret.

# Citations

1. stdin
2. manual:phase-ci-cross-model-qa-and-hardening