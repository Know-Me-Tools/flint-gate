# Plan — agent-authz-budget-rate-limiting

_Generated: 2026-07-09_

## Summary

Assessment found that the server-side budget, rate-limiting, and authorization features
are all complete. The only gap is SDK client methods for the approval admin endpoints
(`GET /approvals`, `GET /approvals/{id}`, `POST /approvals/{id}/decision`). Two
changes, ordered Go-first since Go is the reference SDK — the TypeScript change can
reference the Go implementation for naming consistency.

## Change Order

| # | Change ID | Description | Depends On |
|---|-----------|-------------|------------|
| 1 | `add-go-sdk-approval-methods` | Add `ApprovalStatus`, `ApprovalDecision` types + 3 client methods to Go SDK | — |
| 2 | `add-ts-sdk-approval-methods` | Mirror approval types + 3 `FlintGateAdmin` methods in TypeScript SDK | Change 1 (for naming consistency) |

## Key Constraints

- `ApprovalDecision` serializes as `"approve"` / `"deny"` (`snake_case`) — NOT `"Approve"` / `"Deny"`
- All endpoints are admin-only (hit the `adminUrl`/admin port); test fixture uses loopback
- Integration tests for approval endpoints deferred — require a live stream to generate pending approvals
- No Rust changes; no new infrastructure; no CI workflow changes needed

## Recommended Execution

Run `/kbd-apply add-go-sdk-approval-methods` then `/kbd-apply add-ts-sdk-approval-methods`.
