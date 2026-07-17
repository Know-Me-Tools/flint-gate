# Assessment — sdk-integration-test-expansion

_Generated: 2026-07-09_

## Executive Summary

The existing integration tests cover health/readiness, basic route CRUD (create + list + get + delete), and basic API key lifecycle (create + list + delete). The TypeScript SDK has full policy CRUD admin methods; the **Go SDK has NO policy methods at all** — this is a larger gap than the goals implied. The approval smoke test (goal 4) is trivially addable to both SDKs since `listApprovals` now exists in both.

The phase scope must be extended slightly: to test policy CRUD integration we first need Go SDK policy methods (types + client methods), then both SDKs can be integration-tested together.

---

## Current Integration Test Coverage

### Go (`sdks/go/integration_test.go`)

| Area | Covered | Notes |
|------|---------|-------|
| `GetHealth` | ✅ | |
| `GetReady` | ✅ | |
| Route: create + list + get + delete | ✅ | idempotent delete tested |
| Route: update (`UpsertRoute`) | ❌ | method exists in client.go but not tested |
| API key: create + list + delete | ✅ | |
| Policies: any method | ❌ | **No policy methods exist in Go SDK** |
| Approvals: list smoke | ❌ | method exists (from prior phase) |

### TypeScript (`sdks/typescript/src/__tests__/integration.test.ts`)

| Area | Covered | Notes |
|------|---------|-------|
| `getHealth` | ✅ | |
| `getReady` | ✅ | |
| Route: create + list + get + delete | ✅ | |
| Route: update (`updateRoute`) | ❌ | method exists in admin.ts but not tested |
| API key: create + list + revoke | ✅ | |
| Policies: any method | ❌ | **methods exist in admin.ts but not integration-tested** |
| Approvals: list smoke | ❌ | method exists (from prior phase) |

---

## Codebase Inventory

### Go SDK — policy gap (unexpected finding)

`sdks/go/client.go` has **zero** policy methods. `sdks/go/types.go` has **zero** policy types. The TypeScript SDK added full policy CRUD in a prior phase (`add-sdk-policy-methods` archived 2026-07-09), but the Go SDK was never updated.

**This means goal 1 (policy CRUD integration tests for Go) requires first adding Go SDK policy methods.** This is additional pre-requisite work not originally scoped.

**Go SDK missing policy methods:**
- Types: `PolicyRow`, `PolicyVersionRow`, `PolicyHistoryResponse`, `UpsertPolicyInput`, `UpsertPolicyResponse`, `RollbackResponse`
- Methods: `ListPolicies`, `GetPolicy`, `CreatePolicy`, `UpdatePolicy`, `DeletePolicy`, `GetPolicyHistory`, `RollbackPolicy`

### TypeScript SDK — policy methods exist, no integration tests

`sdks/typescript/src/admin.ts` already has:
- `listPolicies`, `getPolicy`, `createPolicy`, `updatePolicy`, `deletePolicy`
- `getPolicyHistory`, `rollbackPolicy`

Unit tests exist in `admin.test.ts`. No integration tests.

### Server — policy API shapes (from admin/mod.rs + db/mod.rs)

**`GET /policies`** → `{"policies": [PolicyRow, ...]}`  (wrapped array — envelope!)
**`GET /policies/{id}`** → `PolicyRow` (bare)
**`POST /policies`** + **`PUT /policies/{id}`** → `{"status": "created"|"updated", "id": "...", "reloaded": true|false, "warnings": [...]}`
**`DELETE /policies/{id}`** → `{"status": "deleted", "id": "...", "reloaded": true|false}`
**`GET /policies/{id}/history`** → `{"policy_id": "...", "total_hint": null, "versions": [PolicyVersionRow, ...]}`
**`POST /policies/{id}/rollback`** → `{"status": "rolled_back"|"ok", "policy_id": "...", "from_version": N, "to_version": M, "reloaded": true, "warnings": [...]}`

**`PolicyRow` JSON fields:**
```
id: String
policy_text: String
schema_json?: Value (optional)
entities_json?: Value (optional)
enabled: bool
written_by?: String (optional)
```

**`PolicyVersionRow` JSON fields:**
```
id: i32
policy_id: String
version_num: i32
policy_text: String
schema_json?: Value
entities_json?: Value
written_by?: String
written_at: DateTime<Utc> (RFC3339)
```

**CRITICAL envelope:** `GET /policies` returns `{"policies": [...]}` — Go SDK `ListPolicies` MUST unwrap this (same pattern as `ListApprovals`). TypeScript `listPolicies` currently calls `adminRequest<PolicyRow[]>("/policies")` — this is WRONG and will fail against the live server (gets the object, not the array). This is a bug in the existing TS code that integration tests will catch.

### Fixture suitability

`docker-compose.test.yml` + `config.test.yaml` (loopback admin port, AllowLoopback posture) is sufficient for all policy CRUD tests. No additional services needed.

---

## Gap Analysis

| Gap | Severity | Recommendation |
|-----|----------|----------------|
| Go SDK: no policy types or methods | **HIGH** | Add `PolicyRow`, `PolicyVersionRow`, `PolicyHistoryResponse`, `UpsertPolicyInput`, `UpsertPolicyResponse`, `RollbackResponse` types + 7 client methods |
| TypeScript `listPolicies` has envelope bug (gets `{policies:[]}` not `[]`) | **HIGH** | Fix `adminRequest` call to unwrap `{ policies: unknown[] }` |
| Go integration tests: route update missing | MEDIUM | Add `TestIntegration_RouteUpdate` |
| Go integration tests: policy CRUD missing | MEDIUM | Add after Go SDK policy methods exist |
| Go integration tests: approval list smoke missing | LOW | Trivial addition |
| TS integration tests: route update missing | MEDIUM | Add `updateRoute` test |
| TS integration tests: policy CRUD missing | MEDIUM | Add after TS envelope bug fixed |
| TS integration tests: approval list smoke missing | LOW | Trivial addition |

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| `GET /policies` envelope wrapping (same pattern as approvals) | HIGH | Go SDK must unwrap `{"policies":[]}`. TS `listPolicies` currently broken — fix it. |
| Cedar policy text must be valid Cedar syntax | MED | Use `permit(principal, action, resource);` — the simplest valid Cedar policy |
| Rollback requires at least 2 versions to exist | MED | Test must create policy, update it to get v2, then rollback to v1 |
| Policy delete hot-reloads Cedar engine — may take >1s | LOW | Use 30s test timeouts (already the default) |
| Idempotent delete for policies: `DELETE /policies/{id}` returns 404 if not found | LOW | Unlike routes/keys, policy delete may NOT be idempotent — test must not double-delete |

---

## Recommended Changes (Ordered)

| # | Change ID | Description | Scope | Depends On |
|---|-----------|-------------|-------|------------|
| 1 | `add-go-sdk-policy-methods` | Add policy types + 7 client methods to Go SDK | `sdks/go/types.go`, `sdks/go/client.go` | — |
| 2 | `fix-ts-listpolicies-envelope` | Fix `listPolicies` in TypeScript admin.ts to unwrap `{"policies":[]}` envelope | `sdks/typescript/src/admin.ts` | — |
| 3 | `expand-go-integration-tests` | Add Go integration tests: route update, policy CRUD, approval smoke | `sdks/go/integration_test.go` | Change 1 |
| 4 | `expand-ts-integration-tests` | Add TS integration tests: route update, policy CRUD, approval smoke | `sdks/typescript/src/__tests__/integration.test.ts` | Change 2 |

**Total: 4 changes.** Changes 1 and 2 are independent; changes 3 and 4 depend on them respectively.

---

## Out-of-Scope Confirmation

- Approval full-flow integration tests (require live stream fixture — deferred to Option B phase)
- Go SDK: `UpdateRoute` is already in `client.go` (as `UpsertRoute`) — no new method needed, just a test
- Policy validate/simulate endpoints — not in scope for this phase
- Load testing

## Security Constraints (preserved)

- Admin port (4457) stays loopback-bound in config.test.yaml — no exposure
- No secrets committed — test Cedar policy is plain text, no credentials
- Fail-closed preserved — no changes to authorization logic
