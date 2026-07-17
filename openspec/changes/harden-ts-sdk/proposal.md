# Proposal — harden-ts-sdk

**Phase:** post-beta-hardening
**Goal:** G-5 — Agent SDK enhancements (TypeScript)
**Severity:** MEDIUM
**Depends on:** —

## Problem

The TypeScript SDK (`sdks/typescript/src/`) mirrors the Go SDK's
functional gaps: no token refresh, no 429 retry, no structured error
helpers, no SSE reconnect.

## Scope

- `sdks/typescript/src/client.ts` — `TokenProvider` type, `doJSON`
  retry-on-429, error helpers
- `sdks/typescript/src/stream.ts` — SSE reconnect loop
- `sdks/typescript/src/__tests__/` — unit tests

## Out of scope

- Admin API surface changes
- WebSocket reconnect

## Acceptance Criteria

- `pnpm test` passes in `sdks/typescript/`
- 429 mock test: server returns 3×429 then 200; client succeeds
- `isRateLimited`, `isUnauthorized`, `isApprovalRequired` return correct
  booleans for status codes 429, 401, 403
- SSE reconnect test: connection drops; client reconnects and continues
- Static `token` string option still works (backwards-compatible via adapter)
