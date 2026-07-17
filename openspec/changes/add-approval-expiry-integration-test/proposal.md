# add-approval-expiry-integration-test

**Phase:** beta-release-readiness / Phase 2 (Serious gap S-5)

## Problem

The approval TTL janitor (background task that auto-denies expired approvals)
is unit-tested only in memory. There is no integration test verifying the full
path: stream hangs → TTL expires → janitor fires → stream terminates and
`ListApprovals` returns empty. A janitor bug would cause paused streams to
hang indefinitely, consuming resources permanently.

## Solution

Add integration tests for the TTL expiry path in both Go and TypeScript SDKs,
using a test-specific short TTL (5s) configured in `config.test.yaml`.

Changes to `config.test.yaml`:
- Set `approval.ttl_seconds: 5` (short TTL for test speed)
- Set `approval.janitor_interval_seconds: 1` (frequent sweeps)

Go test `TestIntegration_ApprovalExpiry`:
1. Create `@require_approval` Cedar policy
2. Send stream request → approval appears
3. Wait 8 seconds (> TTL)
4. Assert `ListApprovals` returns empty (janitor cleaned up)
5. Assert stream terminated (response body closed)

TypeScript test mirrors the Go test with `{ timeout: 20_000 }`.

## Files to change

- `config.test.yaml` — add short TTL and janitor interval settings
- `sdks/go/integration_test.go` — add `TestIntegration_ApprovalExpiry`
- `sdks/typescript/src/__tests__/integration.test.ts` — add expiry test
