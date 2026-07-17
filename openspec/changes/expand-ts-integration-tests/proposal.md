# expand-ts-integration-tests

**Phase:** sdk-integration-test-expansion
**Scope:** `sdks/typescript/src/__tests__/integration.test.ts`
**Depends on:** `fix-ts-listpolicies-envelope`

## Why

The existing TypeScript integration tests cover health, route CRUD, and API key
lifecycle. Route update, policy CRUD, and the approval list smoke test are not
covered despite the client methods existing.

## What

Add three new `it()` blocks inside the existing `describe.skipIf(!gatewayUrl)` suite:

### 1. Route update test

```typescript
it("updateRoute changes the route match path", async () => {
  const rawId = uniqueId("integ-route-upd");
  await admin.createRoute({
    id: asRouteId(rawId),
    site: asSiteId("test"),
    match: { path: "/integ-upd/**", methods: ["GET"] },
    upstream: "http://127.0.0.1:4457/health",
    enabled: true,
  });
  try {
    const updated = await admin.updateRoute(rawId, {
      id: asRouteId(rawId),
      site: asSiteId("test"),
      match: { path: "/integ-upd-changed/**", methods: ["GET"] },
      upstream: "http://127.0.0.1:4457/health",
      enabled: true,
    });
    expect(updated.match.path).toBe("/integ-upd-changed/**");

    const fetched = await admin.getRoute(rawId);
    expect(fetched.match.path).toBe("/integ-upd-changed/**");
  } finally {
    await admin.deleteRoute(rawId);
  }
});
```

### 2. Policy CRUD test

```typescript
it("policy CRUD: create → update → history → rollback → delete", async () => {
  const policyId = uniqueId("integ-policy");
  const created = await admin.createPolicy({
    id: policyId,
    policy_text: "permit(principal, action, resource);",
    enabled: true,
  });
  expect(["created", "ok"]).toContain(created.status);

  try {
    const fetched = await admin.getPolicy(policyId);
    expect(fetched.policy_text).toBe("permit(principal, action, resource);");

    // listPolicies must include the new policy
    const all = await admin.listPolicies();
    expect(all.some((p) => p.id === policyId)).toBe(true);

    // Update to v2
    const updated = await admin.updatePolicy(policyId, {
      id: policyId,
      policy_text: "forbid(principal, action, resource);",
      enabled: true,
    });
    expect(["updated", "ok"]).toContain(updated.status);

    // History must have >= 2 versions
    const history = await admin.getPolicyHistory(policyId);
    expect(history.versions.length).toBeGreaterThanOrEqual(2);

    // Rollback to version 1
    const rb = await admin.rollbackPolicy(policyId, 1);
    expect(rb.to_version).toBe(1);

    // History now has >= 3 rows (rollback creates a new version)
    const historyAfter = await admin.getPolicyHistory(policyId);
    expect(historyAfter.versions.length).toBeGreaterThanOrEqual(3);

    // Delete
    const del = await admin.deletePolicy(policyId);
    expect(del.status).toBe("deleted");

    // 404 after delete
    await expect(admin.getPolicy(policyId)).rejects.toThrow();
  } catch (e) {
    // Attempt cleanup even on failure — best-effort since delete may already have run
    try { await admin.deletePolicy(policyId); } catch { /* ignore */ }
    throw e;
  }
});
```

### 3. Approval list smoke test

```typescript
it("listApprovals returns an array (empty in clean fixture)", async () => {
  const approvals = await admin.listApprovals();
  expect(Array.isArray(approvals)).toBe(true);
});
```

## Key implementation notes

- Policy delete is NOT idempotent (404 on double-delete) — test must not call `deletePolicy` twice
- The try/finally cleanup pattern used for routes is adapted: catch-cleanup-rethrow since delete is part of the happy path here
- `admin.getPolicyHistory` already returns `PolicyHistoryResponse` with a `.versions` array (TypeScript SDK types are correct)
- `admin.rollbackPolicy(id, 1)` uses version number 1, which is always the first version created

## Verification

- `pnpm typecheck` clean
- `pnpm test` all existing unit tests pass (integration tests remain skipped in unit-only mode)
- Integration tests pass against the live fixture with `INTEGRATION_GATEWAY_URL=http://localhost:4457 pnpm test:integration`
