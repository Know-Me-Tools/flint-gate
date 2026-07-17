# fix-ts-listpolicies-envelope

**Phase:** sdk-integration-test-expansion
**Scope:** `sdks/typescript/src/admin.ts`
**Depends on:** nothing (bug fix only)

## Why

`GET /policies` returns `{"policies": [...]}` — a wrapped envelope, not a bare
array. The current `listPolicies` implementation calls:

```typescript
return this.client.adminRequest<PolicyRow[]>("/policies", { signal });
```

This types the response as `PolicyRow[]` but actually receives `{ policies: PolicyRow[] }`.
The raw response object is returned where an array is expected, causing silent
data-shape mismatch. Integration tests against the live server will fail with
`result.length === undefined` or similar.

This follows the same pattern as `listApprovals` (fixed in the prior phase),
which correctly unwraps `{"approvals": [...]}`.

## What

### Fix `listPolicies` in `sdks/typescript/src/admin.ts`

**Before:**
```typescript
async listPolicies(signal?: AbortSignal): Promise<PolicyRow[]> {
  return this.client.adminRequest<PolicyRow[]>("/policies", { signal });
}
```

**After:**
```typescript
async listPolicies(signal?: AbortSignal): Promise<PolicyRow[]> {
  const body = await this.client.adminRequest<{ policies: PolicyRow[] }>(
    "/policies",
    { signal },
  );
  return body.policies ?? [];
}
```

No type changes needed — `PolicyRow` is already defined correctly in `types.ts`.

## Verification

- `pnpm typecheck` clean
- `pnpm test` all 16 unit tests continue to pass (unit test mocks the full response, not the envelope)
- Integration test `listPolicies()` returns an array (not an object) against the live server
