# Reflection — sdk-integration-test-expansion

_Generated: 2026-07-09_

## Goal Achievement

| # | Goal | Status | Notes |
|---|------|--------|-------|
| 1 | Policy CRUD integration tests (Go + TypeScript) | ✅ MET | `TestIntegration_PolicyCRUD` (Go) and equivalent TypeScript test added; covers create → list → get → update → history (≥2 versions) → rollback (≥3 versions) → delete → 404 verify |
| 2 | Route lifecycle integration tests (update + delete) | ✅ MET | `TestIntegration_RouteUpdate` (Go) and `updateRoute` TS test added; prior tests already covered delete |
| 3 | API key lifecycle integration tests | ✅ MET | Existing tests in both SDKs already covered full lifecycle; no gaps found |
| 4 | Approval list smoke test | ✅ MET | `TestIntegration_ApprovalSmoke` (Go) and `listApprovals` TS test added |
| 5 | CI green (all existing tests continue to pass) | ✅ MET | `go test ./...` and `pnpm test` clean; integration tests correctly skip without gateway URL |

**Overall: 5/5 goals MET (100%)**

---

## Delivered Changes

| # | Change | Files Modified | Outcome |
|---|--------|---------------|---------|
| 1 | `add-go-sdk-policy-methods` | `sdks/go/types.go`, `sdks/go/client.go` | 7 policy types + 7 client methods (ListPolicies, GetPolicy, CreatePolicy, UpdatePolicy, DeletePolicy, GetPolicyHistory, RollbackPolicy); go vet + go test clean |
| 2 | `fix-ts-listpolicies-envelope` | `sdks/typescript/src/admin.ts`, `sdks/typescript/src/__tests__/admin.test.ts` | Fixed silent `{"policies":[]}` envelope bug; updated unit test mock to match real server shape; 16 unit tests pass |
| 3 | `expand-go-integration-tests` | `sdks/go/integration_test.go` | Added `TestIntegration_RouteUpdate`, `TestIntegration_PolicyCRUD`, `TestIntegration_ApprovalSmoke`; go build + go vet clean |
| 4 | `expand-ts-integration-tests` | `sdks/typescript/src/__tests__/integration.test.ts` | Added route update, policy CRUD, approval smoke tests; 16 unit tests pass, 7 integration tests skip cleanly |

---

## Artifact Quality Summary

| Metric | Value |
|--------|-------|
| Changes with QA | 4/4 |
| Verification method | Manual (`go vet`, `go test`, `pnpm typecheck`, `pnpm test`) |
| Unit test regressions | 0 |
| Build failures | 0 |
| Artifact-refiner logs | n/a (no `.refiner/` directory — QA was inline) |

No constraint violations found. All changes were self-contained with clear verification passes after each.

---

## Unscoped Gap Found (Notable)

The assessment surfaced a **pre-requisite gap not in the original goals**: the Go SDK had zero policy types or methods. The TypeScript SDK had added them in a prior phase but the Go equivalent was never written. This required change 1 (`add-go-sdk-policy-methods`) to be added to scope before the integration tests could be written.

This gap existed because the prior `agent-authz-budget-rate-limiting` phase only completed the TypeScript approval SDK methods — the Go SDK parity was noted as a gap in that phase's reflection but not tracked as a concrete next-phase deliverable.

**Pattern to watch:** when a new capability lands in the TypeScript SDK, the Go SDK should receive parity in the same phase or the gap should be explicitly tracked.

---

## Envelope Bug Fixed

A silent correctness bug was found in `sdks/typescript/src/admin.ts`: `listPolicies` called `adminRequest<PolicyRow[]>("/policies")` but the server returns `{"policies": [...]}`. Unit tests never caught this because they mock `adminRequest` to return whatever shape the test passes in — the wrapping layer is invisible to mocks. The fix unwraps the envelope and the unit test was updated to mock the real server response shape.

**Lesson:** for any admin method that calls `GET /<resource>` that returns a named-envelope (like `{"policies":[]}` or `{"approvals":[]}`), the unit test mock must be shaped to match the actual server JSON, not the unwrapped result type.

---

## Technical Debt Introduced

None. The phase added tests and SDK methods with no architectural shortcuts. The `GetPolicyHistory` Go method uses `url.Values` for query-string building (consistent with existing code), and the TypeScript rollback test uses a `find` on the history versions array (robust to ordering).

One minor note: the Go `TestIntegration_PolicyCRUD` uses `t.Cleanup` for the delete but the delete could fail if the test fails after create but before the cleanup is registered. This is a known pattern accepted in the existing tests (`TestIntegration_Routes`, `TestIntegration_APIKeys`) and is acceptable for integration tests against a dev fixture.

---

## Security Constraints (Verified Preserved)

- ✅ Admin port (4457) stays loopback-bound — no changes to config or docker-compose
- ✅ No secrets committed — Cedar policy text is plain `permit(principal, action, resource);`
- ✅ No existing unit tests broken — updated mock in `admin.test.ts` to match real server shape
- ✅ Fail-closed: no changes to authorization or routing logic
- ✅ No new docker-compose services added

---

## Lessons Captured

1. **Mock unit tests can hide envelope bugs.** When `adminRequest` is mocked to return a bare array, a method that forgets to unwrap `{"items":[]}` will pass unit tests but fail against the live server. Mitigation: always shape mocks to match the actual HTTP response body the server sends, not the Go/TS type the SDK method returns.

2. **SDK parity tracking.** When a new admin API surface lands in one SDK (TypeScript in this case), the equivalent Go implementation should happen in the same phase. A convention or a checklist item in the phase template would prevent the drift found here.

3. **Policy delete is not idempotent.** Unlike `DeleteRoute` and `DeleteAPIKey` (which swallow 404), `DeletePolicy` returns a real 404 on a missing policy. Tests must not call delete twice. This is a semantic difference that future contributors need to be aware of.

4. **Integration test cleanup via `t.Cleanup` is the right pattern.** Registering cleanup immediately after a successful create (before any assertions that could fail) means resources are always reclaimed even when the test fails mid-way.

---

## Recommended Next Phase

### Option A — Approval Full-Flow Integration Tests (Option B from prior reflection)

The approval smoke test confirms the list endpoint is reachable. Full end-to-end approval flow (agent makes tool call → flint-gate buffers → admin approves → response unblocked) requires either:
- A stub upstream server that can be scripted to send a streaming tool-call response
- Or direct HTTP injection via the test fixture

This is the highest-value remaining integration gap. It would require a small addition to `docker-compose.test.yml` (a mock upstream) and a test harness that drives both the agent-facing proxy port and the admin API in sequence.

**Complexity:** MEDIUM — new docker-compose service, but the SDK methods and admin API are already fully tested.

### Option B — Rust SDK Parity

The Go and TypeScript SDKs now have full parity across health, routes, API keys, policies, and approvals. If a Rust SDK exists or is planned, it would follow the same pattern.

### Option C — Admin UI Integration Tests (Playwright)

The web admin UI has Playwright scaffolding from the `agent-authz-budget-rate-limiting` phase. E2E tests for the admin UI policy CRUD flows would complement the SDK integration tests and close the last observable gap in test coverage.

**Recommendation: Option A** — the approval full-flow integration test is the most direct proof that the system works end-to-end for its primary use case (human-in-the-loop tool-call gating).
