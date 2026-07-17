# add-sdk-policy-methods

## Summary

Add policy CRUD, history, and rollback methods to the `FlintGateAdmin` TypeScript SDK class. The SDK currently only exposes routes and API-key management; the full Cedar policy admin surface added over the last two phases is unreachable from external callers using the SDK.

## Why

`FlintGateAdmin` is the public TypeScript contract for consuming flint-gate's admin API. Sister projects (notably `flint-platform-agent` if it ever needs to author policies, and any future external integrations) should be able to use the SDK rather than hand-rolling fetch calls. The API surface is stable and fully known; completing the SDK now costs less than retrofitting it after external consumers exist.

## What Changes

### `sdks/typescript/src/types.ts`

Add:
- `PolicyRow` interface (`id`, `policy_text`, `schema_json?`, `entities_json?`, `enabled`, `written_by?`)
- `PolicyVersionRow` interface (`id`, `policy_id`, `version_num`, `policy_text`, `schema_json?`, `entities_json?`, `written_by?`, `written_at`)
- `PolicyHistoryResponse` interface (`policy_id`, `total_hint`, `versions: PolicyVersionRow[]`)
- `RollbackResponse` interface (`status`, `policy_id`, `from_version`, `to_version`)
- `UpsertPolicyInput` interface (`id`, `policy_text`, `schema_json?`, `entities_json?`, `enabled?`)

### `sdks/typescript/src/admin.ts`

Add to `FlintGateAdmin`:
- `listPolicies(signal?)` → `Promise<PolicyRow[]>`
- `getPolicy(id, signal?)` → `Promise<PolicyRow>`
- `createPolicy(input: UpsertPolicyInput, signal?)` → `Promise<{ status: string; id: string; reloaded?: boolean; warnings?: string[] }>`
- `updatePolicy(id, input: UpsertPolicyInput, signal?)` → same return
- `deletePolicy(id, signal?)` → `Promise<{ status: string; id: string }>`
- `getPolicyHistory(id, opts?: { offset?: number; limit?: number }, signal?)` → `Promise<PolicyHistoryResponse>`
- `rollbackPolicy(id, versionNum: number, signal?)` → `Promise<RollbackResponse>`

### `sdks/typescript/src/__tests__/admin.test.ts` (new file or extend existing)

- Unit tests for each new method using `msw` or `undici.MockAgent` for HTTP mocking.
