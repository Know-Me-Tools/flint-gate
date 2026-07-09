# Assessment — approval-full-flow-integration

_Generated: 2026-07-09_

## Executive Summary

The approval infrastructure (Cedar `@require_approval` annotation → `AuthzDecision::RequireApproval`
→ `ApprovalManager` → Admin API `POST /approvals/{id}/decision` → stream unblock) is fully
implemented and well-tested at the unit level. The existing integration tests only call
`ListApprovals` on an idle fixture (smoke test). No end-to-end integration test exists that
exercises the complete flow: an agent request triggering an approval → admin deciding →
the buffered response being released.

**The key constraint**: the approval flow only fires for *streaming* requests on routes with
`stream.enabled: true`, an `Authorize` hook wired to a Cedar policy containing `@require_approval`,
and `approval.enabled: true` (the server default). The current `config.test.yaml` has no such
route and `approval` is not explicitly configured (defaults to enabled with a 300s TTL).

**Approach discovery**: The cleanest path does NOT require a full stub upstream container.
The approval flow can be triggered using the *existing* `http://flint-gate:4457/health`
upstream — it just needs a route that has `stream.enabled: true` and `ag_ui.enabled: true`
(or `ndjson` protocol) plus an Authorize hook + a `@require_approval` Cedar policy. The
integration test can create all of these via the Admin API, then send a crafted streaming
request to the proxy port (4456), race the approval decision, and assert the outcome.

However, the stream response format (AG-UI SSE or ndjson) is non-trivial to parse in a
test. The **simplest viable approach** is:

1. Add a `@require_approval`-annotated Cedar policy via the Admin API
2. Create a route with `stream.enabled: true` and `ag_ui.enabled: true` pointing at a stub
3. Send a minimal SSE request to the proxy port (4456)
4. Assert the approval appears in `ListApprovals` within a timeout
5. Call `DecideApproval` with approve or deny
6. Assert the SSE stream closes (deny) or carries a release event (approve)

**Critical finding**: the fixture needs a minimal HTTP stub that returns a synthetic
AG-UI/SSE streaming response containing a tool call. Without this, the stream processor
never sees a tool call to authorize, so the `@require_approval` policy is never evaluated
at the per-tool level. The health endpoint (`/health`) returns JSON, not SSE — a dedicated
stub is required.

---

## Current State

### Infrastructure

| Component | Status | Notes |
|-----------|--------|-------|
| `ApprovalManager` (Rust) | ✅ Complete | `register`, `decide`, `list`, `status`, `purge_expired` — 14 unit tests |
| Cedar `@require_approval` annotation | ✅ Complete | Maps `AuthzDecision::RequireApproval(ctx)` |
| AG-UI stream processor — approval path | ✅ Complete | `request_approval` → `gate:approval_request` event |
| A2UI stream processor — approval path | ✅ Complete | Same pattern |
| Admin API `GET /approvals` + `POST /approvals/{id}/decision` | ✅ Complete | Tested in SDK unit tests |
| Go SDK: `ListApprovals`, `GetApproval`, `DecideApproval` | ✅ Complete | Added in prior phase |
| TypeScript SDK: `listApprovals`, `getApproval`, `decideApproval` | ✅ Complete | Added in prior phase |
| `approval.enabled` default | ✅ `true` | No config block needed for the default |
| Integration test fixture: `docker-compose.test.yml` | Sufficient with addition | Needs a stub HTTP server emitting SSE with a tool call |
| Integration test: approval full-flow | ❌ Missing | This is the phase deliverable |

### How the Flow Is Triggered (Key Finding)

To trigger an approval in a streaming context:

1. A route must have `stream.enabled: true` and `ai.ag_ui.enabled: true` (for AG-UI protocol)
   or `protocol: ndjson` (simpler to parse in tests)
2. The route must have an `Authorize` pre-request hook
3. A Cedar policy loaded via `POST /policies` must contain `@require_approval("reason")` on a
   `permit` rule that matches the request
4. The upstream must return a response the stream processor interprets as containing a tool call
5. When the processor sees the tool call + `RequireApproval` decision, it registers with the
   `ApprovalManager` and emits a `gate:approval_request` SSE event, then pauses

**Per-tool authorization context**: the `Authorize` hook evaluates the *route-level* request;
per-tool Cedar evaluation happens inside the stream processor for each tool call event in the
response. The Cedar entities for per-tool calls use `action: "invoke"` and `resource` set to
the tool name.

### Stub Upstream Requirement

The fixture needs a stub that:
- Accepts `POST /` (or any path the test route proxies to)
- Returns `Content-Type: text/event-stream`
- Emits a minimal AG-UI event sequence containing a `ToolCallStart` + `ToolCallArgs` + `ToolCallEnd`
  sequence (or ndjson equivalent) for a tool call named e.g. `integ_test_tool`

**Simplest viable stub**: a single-binary Go HTTP server included as a new service in
`docker-compose.test.yml`. A static response handler — no dynamic scripting required.
The tool call can be hardcoded (the approval Cedar policy can match it by tool name).

Alternatively: a `node:alpine` container running a one-file `server.mjs` that streams
the hardcoded ndjson or SSE response. This avoids a Go build step in the fixture.

---

## Gap Analysis

| Gap | Severity | Description |
|-----|----------|-------------|
| Mock upstream streaming server | **HIGH** | New docker-compose service returning a tool-call SSE or ndjson response |
| Cedar `@require_approval` policy wired to the test tool | **HIGH** | Created via Admin API at test start — no config file change needed |
| Integration test route with `stream.enabled: true` + AG-UI/ndjson | **HIGH** | Created via Admin API (test creates it, cleanup deletes it) |
| Go approval full-flow integration test | **HIGH** | `TestIntegration_ApprovalFlow` |
| TypeScript approval full-flow integration test | **HIGH** | `approval full-flow round-trip` |
| CI workflow update for new docker-compose service | MED | `integration.yml` may need no changes if docker-compose handles it |

---

## Architecture Decision: Stub Upstream

### Option 1 — Node.js one-file stub (recommended)

```yaml
# docker-compose.test.yml addition
mock-upstream:
  image: node:20-alpine
  command: node /stub/server.mjs
  volumes:
    - ./test/stub:/stub:ro
  ports:
    - "9999:9999"
```

`test/stub/server.mjs` serves a hardcoded ndjson stream with a tool call event.
No build step. Node 20 is already used for the TypeScript SDK in CI.
The stub is tiny (~30 lines).

**ndjson format** is simpler to emit and parse than full AG-UI SSE — the test
can scan the raw response body for expected event strings.

### Option 2 — Go stub binary

A `cmd/stub-upstream/main.go` compiled into the flint-gate image or built as a
separate service. More complex but uses the same Go toolchain already in CI.

**Recommendation: Option 1 (Node stub)** — lower CI complexity, no Rust/Go build
in the stub layer, aligned with existing Node.js investment.

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Approval poll timeout in test (network latency) | MED | Register approval then poll `ListApprovals` with a 10s timeout loop |
| Stream response races the test's `DecideApproval` call | MED | Test must see the approval in `ListApprovals` before calling decide |
| Cedar per-tool evaluation context (principal/action/resource) | MED | Policy must use `permit(principal, action, resource)` with `@require_approval` — no entity matching required |
| Node stub service startup latency | LOW | `depends_on: mock-upstream: condition: service_healthy` with a simple `wget` healthcheck |
| Two-replica isolation (approval only on the replica that received the stream) | LOW | Single-replica test fixture — no concern |
| `approval.enabled` config in test | LOW | Default is `true` — no `config.test.yaml` change required |

---

## Recommended Changes

| # | Change ID | Description | Depends On |
|---|-----------|-------------|------------|
| 1 | `add-mock-upstream-stub` | Add Node.js stub server to `docker-compose.test.yml` + `test/stub/server.mjs` emitting ndjson tool call | — |
| 2 | `add-go-approval-flow-test` | Add `TestIntegration_ApprovalFlow` to `sdks/go/integration_test.go` | Change 1 |
| 3 | `add-ts-approval-flow-test` | Add approval full-flow test to `sdks/typescript/src/__tests__/integration.test.ts` | Change 1 |

**Total: 3 changes.** Change 1 is the prerequisite; Changes 2 and 3 are independent of each other.

---

## Security Constraints (Verified Preserved)

- Admin port (4457) stays loopback-bound in `config.test.yaml` — stub upstream talks to flint-gate's proxy port (4456), not admin
- No secrets committed — Cedar policy text, stub ndjson, and route config are all plain text
- Fail-closed semantics preserved — `RequireApproval` without a decision before TTL auto-denies
- No changes to production configuration files or authorization logic

---

## Out of Scope

- Admin UI E2E tests for the approval flow (Option C from reflection)
- Rust SDK parity (Option B)
- Load / stress testing of the approval buffer
- Testing the `CapExceeded` path in integration (unit tests cover it)
- Multi-replica approval routing (in-memory per-replica design — covered by unit tests)
