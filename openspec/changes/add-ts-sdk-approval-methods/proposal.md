# add-ts-sdk-approval-methods

**Phase:** agent-authz-budget-rate-limiting
**Scope:** `sdks/typescript/src/types.ts`, `sdks/typescript/src/admin.ts`
**Depends on:** `add-go-sdk-approval-methods` (for naming consistency reference)

## Why

The TypeScript SDK's `FlintGateAdmin` class has no approval methods. Mirrors the
Go SDK gap: operators cannot list, inspect, or decide pending approvals via TypeScript.

## What

### 1. Types (`sdks/typescript/src/types.ts`)

```typescript
export interface ApprovalStatus {
  readonly approvalId: string;       // JSON: "approval_id"
  readonly principalId: string;      // JSON: "principal_id"
  readonly action: string;
  readonly resourceId: string;       // JSON: "resource_id"
  readonly reason?: string;
  readonly expiresAt: string;        // ISO-8601 datetime string
  readonly expired: boolean;
}

export type ApprovalDecision = "approve" | "deny";
```

Note: camelCase property names in TypeScript; the admin request sends/receives
snake_case JSON from the server. Use a `normalizeApproval()` helper to map
`approval_id` → `approvalId`, etc. (same pattern as `normalizeRoute()` in `admin.ts`).

### 2. `FlintGateAdmin` methods (`sdks/typescript/src/admin.ts`)

```typescript
/** List all non-expired pending approvals. */
async listApprovals(signal?: AbortSignal): Promise<ApprovalStatus[]>

/** Get a single pending approval by id. Throws on 404 (not found / resolved). */
async getApproval(id: string, signal?: AbortSignal): Promise<ApprovalStatus>

/** Approve or deny a pending approval. Throws on 404 (not found) or 410 (expired). */
async decideApproval(id: string, decision: ApprovalDecision, signal?: AbortSignal): Promise<void>
```

## Verification

- `pnpm typecheck` (`tsc --noEmit`) clean
- `pnpm test` (existing unit tests) still pass
- `pnpm test:integration` (no new integration tests for approvals — deferred)
