# Assessment — agent-authz-budget-rate-limiting

_Generated: 2026-07-09_

## Executive Summary

The server-side implementation for agent authorization budgets, rate limiting, and
approval flows is **substantially complete** — all three features are implemented in
the Rust gateway core. The gap is entirely in the **SDK layer**: neither the Go SDK
nor the TypeScript SDK exposes client methods for the approval endpoints that already
exist on the admin API. The goals.md was seeded from the reflection's "Option A"
recommendation; after inspecting the branch, the actual work is narrower than the
goals implied.

---

## Codebase Inventory

### Feature 1: Budget Enforcement — COMPLETE (server-side)

| Component | File | Status |
|-----------|------|--------|
| `MaxTokenBudgetConfig` + `BudgetWindow` enum | `config/types.rs` | ✅ Lifetime + Minute/Hour/Day windowed |
| Windowed enforcement logic | `middleware/pipeline.rs` | ✅ `collect_windowed_budgets()`, `resolve_budget_usage()` |
| Redis-backed atomic window counter | `ratelimit/mod.rs` (`redis-l2` feature) | ✅ Lua-script increment |
| Postgres fallback | `db/mod.rs` → `get_user_token_total_windowed()` | ✅ |
| `usage_events` post-response metering | `middleware/pipeline.rs` | ✅ |

Spec: `openspec/specs/rate-limiting/spec.md` — all requirements implemented.

**SDK gap**: Neither Go nor TypeScript SDK has a `GetBudgetStatus` / `CheckBudget`
method. This is intentional (budget is enforced server-side and surfaced via 429
responses) — there is no "budget status query" endpoint on the admin API. **No SDK
work needed for budgets.**

### Feature 2: Rate Limiting — COMPLETE (server-side)

| Component | File | Status |
|-----------|------|--------|
| `RateLimitConfig` (`per_second`, `burst`, `require_shared_backend`) | `config/types.rs` | ✅ |
| In-process governor (`tower_governor`) | `ratelimit/governor_layer.rs` | ✅ `CredentialKeyExtractor` + IP fallback |
| Redis cross-replica limiter | `ratelimit/mod.rs` (`redis-l2` feature) | ✅ feature-gated |
| OAuth rate limit with `require_shared_backend` | `config/types.rs`, middleware | ✅ |

Spec: `openspec/specs/rate-limiting/spec.md` — all scenarios covered.

**No SDK or integration test work needed for rate limiting** — it surfaces as 429
HTTP responses which are already handled by the existing SDK error path.

### Feature 3: Per-Tool Authorization — COMPLETE (server-side)

| Component | File | Status |
|-----------|------|--------|
| Buffer-until-authorized streaming | `stream/ag_ui.rs`, `stream/a2ui.rs` | ✅ |
| Cedar policy evaluation per tool call | `policy/engine.rs` | ✅ |
| Tool listing visibility filtering | `stream/` | ✅ |
| End-to-end approval pause → resume/deny flow | `stream/ag_ui.rs:245` comment fixed | ✅ |

Spec: `openspec/specs/tool-authorization/spec.md` — all scenarios implemented.

### Feature 4: Approval Admin API — COMPLETE (server-side)

All three REST endpoints are implemented and tested in
`crates/flint-gate-core/src/admin/mod.rs`:

| Endpoint | Handler | Status |
|----------|---------|--------|
| `GET /approvals` | `list_approvals_handler` | ✅ |
| `GET /approvals/{id}` | `get_approval_handler` | ✅ 404 on missing/resolved |
| `POST /approvals/{id}/decision` | `decide_approval_handler` | ✅ 410 on expired |

`ApprovalStatus` shape (from `approval/mod.rs`):
```
approval_id: String
principal_id: String
action: String
resource_id: String
reason: Option<String>
expires_at: DateTime<Utc>
expired: bool
```

`ApprovalDecision`: `Approve` | `Deny`

### Feature 5: SDK Approval Methods — MISSING (THE ONLY REAL GAP)

Neither the Go SDK (`sdks/go/client.go`, `sdks/go/types.go`) nor the TypeScript SDK
(`sdks/typescript/src/admin.ts`, `sdks/typescript/src/types.ts`) exposes ANY method
for the approval endpoints. Zero approval types exist in either SDK.

**Go SDK missing:**
- `ApprovalStatus` struct (type)
- `ApprovalDecision` type (`"approve"` | `"deny"`)
- `ListApprovals(ctx) ([]ApprovalStatus, error)`
- `GetApproval(ctx, id) (ApprovalStatus, error)`
- `DecideApproval(ctx, id, decision) error`

**TypeScript SDK missing:**
- `ApprovalStatus` interface (type)
- `ApprovalDecision` type
- `listApprovals(signal?)` → `Promise<ApprovalStatus[]>`
- `getApproval(id, signal?)` → `Promise<ApprovalStatus>`
- `decideApproval(id, decision, signal?)` → `Promise<void>`

### Feature 6: Integration Tests for Approval Endpoints — MISSING

`sdks/go/integration_test.go` and `sdks/typescript/src/__tests__/integration.test.ts`
were written in the prior phase. Neither has test cases for the approval endpoints.

However, the approval flow requires a **live stream with a paused tool call** to
generate a pending approval — it cannot be tested in isolation against the admin API
(there must be a pending approval registered by the stream processor to list/decide
it). This makes integration testing significantly more complex than routes/keys:

- Option A: Test only the "no pending approvals" list path (trivial — just verifies
  the endpoint responds; not very valuable)
- Option B: Drive a real AG-UI stream that pauses, then approve it via the admin SDK
  — requires an upstream LLM stub or a fixture that emits synthetic tool-call events
  (complex; deferred)

**Decision**: Integration tests for approvals are **deferred**. The complexity
(upstream stub + stream + approval timing) exceeds the value this phase can deliver.
Admin endpoint unit tests already exist in `admin/mod.rs` tests.

### Pending OpenSpec Changes (Stale — Already Delivered)

Two changes remain in `openspec/changes/` with all tasks marked `[x]` but not
archived. They belong to the prior `agent-approval-and-step-up-flows` phase:

| Change | Tasks | Status |
|--------|-------|--------|
| `add-pending-approvals-surface` | 6/6 `[x]` | DONE — code shipped in prior commit |
| `fix-approval-flow-comments-and-verify` | 3/3 `[x]` | DONE — comments fixed + E2E test added |

These should be archived, not re-implemented. They are OUT OF SCOPE for this phase.

---

## Gap Analysis

| Gap | Severity | Recommendation |
|-----|----------|----------------|
| Go SDK: no approval types or methods | HIGH | Implement `ApprovalStatus`, `ApprovalDecision`, 3 methods |
| TypeScript SDK: no approval types or methods | HIGH | Mirror Go SDK; add to `FlintGateAdmin` |
| Integration tests for approval endpoints | LOW | Defer — requires live stream; admin unit tests cover the endpoint |
| Stale OpenSpec changes not archived | LOW | Archive in this phase or leave (cosmetic) |

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| `ApprovalDecision` serialization mismatch (Go enum vs JSON string) | MED | Check server response shape: `"decision": "Approve"` vs `"approve"` |
| TS `decideApproval` needs to know exact JSON field name for `decision` | MED | Read `ApprovalDecisionRequest` struct's `#[serde]` annotations |
| `expires_at` type: DateTime string in JSON vs Go `time.Time` | LOW | Server returns RFC3339; Go `time.Time` handles this natively |
| Integration test attempt creates approval race condition | HIGH | Don't attempt live-stream integration tests this phase |

---

## Recommended Changes (Ordered)

| # | Change ID | Description | Scope |
|---|-----------|-------------|-------|
| 1 | `add-go-sdk-approval-methods` | Add `ApprovalStatus`, `ApprovalDecision` types + 3 client methods to Go SDK | `sdks/go/types.go`, `sdks/go/client.go` |
| 2 | `add-ts-sdk-approval-methods` | Mirror approval types + 3 methods in TypeScript `FlintGateAdmin` + `types.ts` | `sdks/typescript/src/admin.ts`, `types.ts` |

**Total: 2 changes.** Both are small, targeted, and independent. No Rust changes
needed. No new infrastructure. The server already does the work.

---

## Out-of-Scope Confirmation

Per reassessment of the branch state:

- **Budget enforcement** — already complete in Rust; no SDK gap (no budget status
  query endpoint exists or is needed)
- **Rate limiting** — already complete; no SDK gap (surfaces as 429 responses)
- **Per-tool authorization** — already complete; no SDK gap
- **Approval stream flow** — already complete; E2E test exists
- **Integration tests for approval admin endpoints** — deferred (requires live stream fixture)
- **Stale OpenSpec change archival** — cosmetic; not blocking

## Security Constraints (preserved)

- Never expose admin server (port 4457) to public internet — approval endpoints are
  admin-only, loopback-bound in the test fixture; no change needed
- Never commit secrets — no new secrets introduced
- Fail-closed — `ApprovalDecision::Deny` is the fail path; no silent-allow possible
  through the SDK methods
