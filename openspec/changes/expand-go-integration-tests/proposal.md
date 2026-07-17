# expand-go-integration-tests

**Phase:** sdk-integration-test-expansion
**Scope:** `sdks/go/integration_test.go`
**Depends on:** `add-go-sdk-policy-methods`

## Why

The existing Go integration tests cover health, basic route CRUD, and basic API
key lifecycle. Three areas have zero integration coverage despite client methods
existing: route update, policy CRUD, and the approval list smoke test.

## What

Add three new test functions to `sdks/go/integration_test.go`:

### 1. `TestIntegration_RouteUpdate`

Tests `UpsertRoute` (PUT /routes/{id}):
- Create a route with path `/integ-update/**`
- Call `UpsertRoute` to change the path to `/integ-updated/**`
- Call `GetRoute` and verify the path changed
- Cleanup via `DeleteRoute`

### 2. `TestIntegration_PolicyCRUD`

Tests the full policy lifecycle:
- `CreatePolicy` with `permit(principal, action, resource);` Cedar text → assert status "created" or "ok"
- `GetPolicy` by id → assert `PolicyText` matches
- `GetRoutes` → not needed, but verify Cedar policy in `ListPolicies` result
- `UpdatePolicy` with `forbid(principal, action, resource);` → assert status "updated" or "ok"
- `GetPolicyHistory` → assert at least 2 version rows exist
- `RollbackPolicy` to version 1 → assert `FromVersion >= 2`, `ToVersion == 1`
- `GetPolicyHistory` after rollback → assert 3+ version rows (rollback creates a new row)
- `DeletePolicy` → assert status "deleted"
- `GetPolicy` → assert 404 error

Cedar policy text to use:
```
permit(principal, action, resource);
```
(Simplest valid Cedar policy; always parses successfully)

Policy ID convention: `uniqueID("integ-policy")` — same `uniqueID` helper already in the file.

### 3. `TestIntegration_ApprovalSmoke`

Tests `ListApprovals` against live server:
- Call `ListApprovals` (no pending approvals in a clean fixture)
- Assert no error returned
- Assert result is a non-nil slice (may be empty — that is correct)

## Key implementation notes

- `DeletePolicy` is NOT idempotent — do not call it twice; use `t.Cleanup` for one call only
- Use `errors.As(err, &ae) && ae.StatusCode == 404` pattern (already imported via `IsNotFound`) to verify 404 after delete
- All new tests use `30*time.Second` context timeout (consistent with existing tests)
- Policy history asserts `len(history.Versions) >= 2` after create + update

## Verification

- `GOROOT=/opt/homebrew/opt/go/libexec go build -tags integration ./...` compiles
- `GOROOT=/opt/homebrew/opt/go/libexec go vet -tags integration ./...` clean
- Unit tests `go test ./...` continue to pass (build tag gates integration tests)
