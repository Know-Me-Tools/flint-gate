# Plan — sdk-integration-test-expansion

_Generated: 2026-07-09_

## Summary

Assessment surfaced one unscoped gap: the Go SDK has zero policy methods — the TypeScript SDK added them in a prior phase but the Go equivalent was never written. Assessment also found a bug in the existing TypeScript `listPolicies` that the live server would expose (missing envelope unwrap).

The plan is 4 changes in 2 parallel streams:

**Stream A (Go):** `add-go-sdk-policy-methods` → `expand-go-integration-tests`
**Stream B (TypeScript):** `fix-ts-listpolicies-envelope` → `expand-ts-integration-tests`

Changes 1 and 2 are independent and can be applied in either order. Changes 3 and 4 depend on their respective Stream predecessor.

## Change Order

| # | Change ID | Description | Depends On |
|---|-----------|-------------|------------|
| 1 | `add-go-sdk-policy-methods` | Add policy types + 7 client methods to Go SDK | — |
| 2 | `fix-ts-listpolicies-envelope` | Fix `listPolicies` envelope unwrap bug in TypeScript SDK | — |
| 3 | `expand-go-integration-tests` | Add route update, policy CRUD, approval smoke tests to Go | Change 1 |
| 4 | `expand-ts-integration-tests` | Add route update, policy CRUD, approval smoke tests to TypeScript | Change 2 |

## Key Constraints

- `GET /policies` returns `{"policies": [...]}` — MUST unwrap in both SDKs (same pattern as `/approvals`)
- Cedar policy text `permit(principal, action, resource);` is the simplest valid Cedar policy for tests
- Policy delete is NOT idempotent (unlike routes/keys) — do not call twice in tests
- Rollback test requires at least 2 policy versions: create (v1) → update (v2) → rollback to v1
- `GOROOT=/opt/homebrew/opt/go/libexec` prefix required for `go vet` on this machine (broken symlink)
- No new docker-compose services needed — existing fixture is sufficient

## Recommended Execution

```
/kbd-apply add-go-sdk-policy-methods     # change 1 of 4
/kbd-apply fix-ts-listpolicies-envelope  # change 2 of 4
/kbd-apply expand-go-integration-tests   # change 3 of 4
/kbd-apply expand-ts-integration-tests   # change 4 of 4
```
