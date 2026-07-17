# Proposal — fix-approval-expiry-ci

**Phase:** post-beta-hardening
**Goal:** G-2 — CI integration tests (TTL fixture gap)
**Severity:** LOW
**Depends on:** —

## Problem

`integration.yml` wires Go and TypeScript integration tests to CI, but:
1. `docker-compose.test.yml` may not pass the short-TTL config
   (`ttl_seconds: 5`) to the gateway container; if it uses the default
   300s TTL the Go `TestIntegration_ApprovalExpiry` test will timeout
   in CI's 120s window.
2. The TypeScript SDK has no equivalent approval expiry integration test.

## Scope

- `docker-compose.test.yml` — confirm/add short-TTL config mount or env var
- `sdks/typescript/src/__tests__/integration.test.ts` — add expiry test
- `config.test.yaml` — confirm `ttl_seconds: 5`, `janitor_interval_seconds: 1`

## Out of scope

- Changes to `integration.yml` workflow (already correctly wired)
- Changes to Go SDK integration tests (already complete)

## Acceptance Criteria

- `docker-compose.test.yml` passes `ttl_seconds: 5` to the gateway
  (via config mount or `FLINT_APPROVAL_TTL_SECONDS` env var)
- TypeScript test: registers approval, sleeps > 5s, asserts status = `denied`
- Test completes within 30s
- `pnpm test:integration` passes locally against the docker-compose stack
