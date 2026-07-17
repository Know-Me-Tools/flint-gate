# Proposal — harden-go-sdk

**Phase:** post-beta-hardening
**Goal:** G-5 — Agent SDK enhancements (Go)
**Severity:** MEDIUM
**Depends on:** —

## Problem

The Go SDK (`sdks/go/`) is functional but minimal. Production agent
frameworks need:
- Automatic JWT refresh when the token expires (currently callers must
  handle 401 themselves)
- Automatic retry with backoff on 429 responses
- Structured error helpers beyond `IsNotFound`
- SSE reconnect on network drops

## Scope

- `sdks/go/client.go` — `TokenSource` interface, `doJSON` retry-on-429,
  error helpers
- `sdks/go/stream.go` — SSE reconnect loop
- `sdks/go/client_test.go` (new or existing) — unit tests

## Out of scope

- Admin API surface changes (covered by existing methods)
- WebSocket reconnect (lower priority; separate change if needed)

## Acceptance Criteria

- `go test ./...` passes in `sdks/go/`
- 429 mock test: server returns 3×429 then 200; client succeeds
- `IsRateLimited`, `IsUnauthorized`, `IsApprovalRequired` return correct
  values for status codes 429, 401, 403 respectively
- SSE reconnect test: connection drops mid-stream; client reconnects and
  continues delivering events
- Static `Token` string option still works (backwards-compatible)
