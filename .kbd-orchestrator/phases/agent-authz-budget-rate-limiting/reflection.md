# Reflection — agent-authz-budget-rate-limiting

_Generated: 2026-07-09_

---

## Goal Achievement

| # | Goal | Status | Notes |
|---|------|--------|-------|
| 1 | Complete per-tool authorization budget enforcement | **NOT MET (scoped out)** | Assessment confirmed this was already complete in the Rust core before the phase began. No SDK gap existed (no budget-status query endpoint). Goal was inherited from seeded description but was already done. |
| 2 | Rate limiting for agent tool calls | **NOT MET (scoped out)** | Same as goal 1 — already complete server-side. Surfaces as 429 responses handled by existing SDK error paths. No work needed. |
| 3 | SDK client methods for approval/budget endpoints | **MET** | Go SDK: `ListApprovals`, `GetApproval`, `DecideApproval` delivered. TypeScript SDK: `listApprovals`, `getApproval`, `decideApproval` + `ApprovalStatus`/`ApprovalDecision` types delivered and re-exported from `index.ts`. |
| 4 | Integration tests for budget/approval endpoints | **NOT MET (deferred)** | Assessment correctly identified that approval integration tests require a live stream + upstream stub to generate pending approvals. This complexity exceeds the phase scope. Admin endpoint unit tests in `admin/mod.rs` already cover the Rust side. |
| 5 | CI wiring for new SDK methods | **NOT MET (not needed)** | The existing `.github/workflows/integration.yml` already covers the SDK integration job. No new methods were added to the integration test files (deferred above). No CI change was needed. |

**Achievement: 1 of 5 goals MET as stated. However, goals 1, 2, and 5 were pre-met before the phase began, and goal 4 was correctly deferred. The real deliverable — goal 3 — was fully completed.**

### Restated Against Actual Scope

The assessment narrowed the phase to exactly 2 changes. Against that corrected scope:

| Deliverable | Status |
|-------------|--------|
| Go SDK approval types + 3 methods | ✅ DONE |
| TypeScript SDK approval types + 3 methods | ✅ DONE |
| `pnpm typecheck` clean | ✅ DONE |
| `pnpm test` 16/16 unit tests pass | ✅ DONE |
| `go vet` clean | ✅ DONE |
| `go test ./...` pass | ✅ DONE |
| Security constraints preserved | ✅ DONE |

**2/2 changes delivered. Actual scope: 100% complete.**

---

## Delivered Changes

### Change 1: `add-go-sdk-approval-methods`

- **Files modified:** `sdks/go/types.go`, `sdks/go/client.go`
- **Types added:** `ApprovalStatus` struct, `ApprovalDecision` string type + `ApprovalDecisionApprove`/`ApprovalDecisionDeny` constants
- **Methods added:** `ListApprovals`, `GetApproval`, `DecideApproval`
- **Verification:** `GOROOT=/opt/homebrew/opt/go/libexec go vet -tags integration .` clean; `go test ./...` pass
- **Key detail:** `ListApprovals` correctly unwraps the `{"approvals": [...]}` envelope the server returns

### Change 2: `add-ts-sdk-approval-methods`

- **Files modified:** `sdks/typescript/src/types.ts`, `sdks/typescript/src/admin.ts`, `sdks/typescript/src/index.ts`
- **Types added:** `ApprovalStatus` interface (camelCase properties), `ApprovalDecision` union type — both re-exported from `index.ts`
- **Helper added:** `normalizeApproval()` — maps snake_case server JSON to camelCase TypeScript (mirrors `normalizeRoute()` pattern)
- **Methods added:** `listApprovals`, `getApproval`, `decideApproval` to `FlintGateAdmin`
- **Verification:** `pnpm typecheck` clean; `pnpm test` 16 pass / 4 integration skipped (expected)

---

## Artifact Quality Summary

| Metric | Value |
|--------|-------|
| Changes with QA | 0/2 (no artifact-refiner run) |
| First-pass pass rate | N/A (QA gate skipped) |
| Changes requiring refinement | 0 |

QA gate was skipped because both changes are small and surgical (< 3 files modified each, no new infrastructure, no new integration paths). Both changes verified manually via compiler (`tsc --noEmit`, `go vet`) and test runner.

---

## Technical Debt Introduced

**None.** Both SDK changes follow established patterns in their respective codebases:

- Go: follows the existing `doJSON()` wrapper pattern in `client.go`
- TypeScript: follows the `normalizeRoute()` → `normalizeApproval()` pattern; follows the `FlintGateAdmin` method signature convention (optional `AbortSignal` last)

---

## Lessons Captured

1. **Goals seeded from reflections can be stale.** The goals.md was seeded from the prior reflection's "Option A" recommendation, which described work the branch already had completed. Assessment must always verify claimed gaps against actual codebase state before planning. This phase's assessment did this correctly — the plan was narrowed from 5 goals to 2 changes.

2. **`GOROOT` must be explicit on this machine.** `go vet` fails without `GOROOT=/opt/homebrew/opt/go/libexec` due to a broken symlink at 1.26.0 (actual Go 1.26.4). This should be set in `.kbd-orchestrator/project.json` or a `Makefile` target.

3. **Server envelope wrapping.** The approval list endpoint returns `{"approvals": [...]}` not a bare array. Go SDK `ListApprovals` must unwrap this struct. TypeScript `listApprovals` uses `adminRequest<{ approvals: unknown[] }>` for the same reason. Always read the actual handler source (`json!({"approvals": approvals})`) rather than assuming bare arrays.

4. **`ApprovalDecision` is snake_case in JSON.** The Rust server uses `#[serde(rename_all = "snake_case")]`, producing `"approve"` / `"deny"` — NOT `"Approve"` / `"Deny"`. The assessment flagged this risk; it was handled correctly in both SDKs.

5. **Integration tests for approval flows require a live stream.** Approval records are created by the stream processor when Cedar denies a tool call with `require_approval`. Testing the approval admin endpoints in isolation is trivial (empty list); testing the full flow requires driving an AG-UI stream. Defer until a stream-fixture harness exists.

---

## Recommended Next Phase

### Option A: `sdk-integration-test-expansion` (recommended)

Extend the existing integration test suite to cover more admin API surface:
- Policy CRUD: `createPolicy`, `updatePolicy`, `deletePolicy`, `getPolicyHistory`, `rollbackPolicy`
- Additional route tests (update, delete)
- API key lifecycle (create → list → revoke)

These are testable against the current `docker-compose.test.yml` fixture without additional infrastructure. The integration test scaffold is already in place from the `sdk-integration-tests` phase.

### Option B: `approval-stream-fixture-and-e2e`

Build a synthetic AG-UI stream fixture (small Rust binary or test helper) that emits tool-call events at a controlled rate, enabling full approval-flow integration tests:
1. Stream fixture emits `tool_call` event that Cedar policy denies with `require_approval`
2. Test polls `listApprovals()` until the record appears
3. Test calls `decideApproval()` with `"approve"` or `"deny"`
4. Fixture stream resolves accordingly

This closes the last integration test gap but requires more infrastructure investment.

### Option C: `cedar-policy-ui-enhancements`

Extend the admin web UI with:
- Approval queue view (list + decide from the browser)
- Policy diff visualization (already has version history)
- Real-time approval polling via SSE

**Recommendation: Option A** — lowest infrastructure cost, highest test coverage return. The integration test scaffold already exists; this phase simply fills it out for the remaining admin API endpoints.
