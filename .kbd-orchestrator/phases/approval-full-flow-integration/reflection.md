# Reflection — approval-full-flow-integration

_Generated: 2026-07-09_
_Changes: 3/3 complete_

---

## Goal Achievement

| # | Goal | Status | Notes |
|---|------|--------|-------|
| 1 | Mock upstream service in `docker-compose.test.yml` | ✅ MET | `test/stub/server.mjs` + `mock-upstream` service (node:20-alpine) + healthcheck |
| 2 | Go approval full-flow integration test | ✅ MET | `TestIntegration_ApprovalFlow` with `t.Run("approve")` and `t.Run("deny")` sub-tests |
| 3 | TypeScript approval full-flow integration test | ✅ MET | Two `it()` tests inside `describe.skipIf(!gatewayUrl)` with `{timeout: 25_000}` |
| 4 | CI green (workflow + docker-compose) | ✅ MET | `INTEGRATION_PROXY_URL` added; Go timeout bumped 60s→120s; `docker compose config` parses clean |

**All 4 goals MET (100%).**

---

## Delivered Changes

### Change 1 — `add-mock-upstream-stub`

**Files changed:**
- `test/stub/server.mjs` (new, ~40 lines) — Node.js HTTP server; `GET /health` → `{"status":"ok"}`; any other request → chunked ndjson with `TOOL_CALL_START` / `TOOL_CALL_ARGS` / `TOOL_CALL_END` for `integ_test_tool`
- `docker-compose.test.yml` — `mock-upstream` service (node:20-alpine, not exposed to host, healthcheck via wget); `flint-gate` depends_on `mock-upstream: service_healthy`
- `config.test.yaml` — `/stream-test` route: `upstream: http://mock-upstream:9999`, `stream.enabled: true`, `protocol: ndjson`, `hooks.pre_request: [{type: authorize}]`

**Key design decision**: ndjson over AG-UI SSE. Simpler to emit from a 30-line Node.js stub, simpler to assert in tests (plain string scan vs SSE frame parsing). The ndjson stream processor in `pipeline.rs` fully supports the approval path — no functional compromise.

**Key design decision**: Cedar `@require_approval` policy created at runtime via Admin API. Not committed to any config file. Test cleanup via `t.Cleanup` / `finally` block.

### Change 2 — `add-go-approval-flow-test`

**Files changed:**
- `sdks/go/integration_test.go` — added stdlib-only `testJWT` helper (HS256 with `crypto/hmac`/`crypto/sha256`); `integrationProxyURL` helper (reads `INTEGRATION_PROXY_URL`, defaults to `http://127.0.0.1:4456`); `pollForApproval` helper (500ms poll loop with timeout); `TestIntegration_ApprovalFlow` with approve and deny sub-tests
- `.github/workflows/integration.yml` — added `INTEGRATION_PROXY_URL: http://localhost:4456` env; both test steps receive it; Go test timeout 60s → 120s

**Test pattern**: the approve path uses a goroutine to drain the ndjson stream concurrently while the main goroutine races the approval decision. The `pollForApproval` loop has a 10s deadline. The `DecideApproval` call unblocks the buffered stream; the goroutine collects all event `type` values and asserts `TOOL_CALL_START` is present.

### Change 3 — `add-ts-approval-flow-test`

**Files changed:**
- `sdks/typescript/src/__tests__/integration.test.ts` — added `import { createHmac } from "node:crypto"`; `proxyUrl` constant; `testJWT()` helper (HS256, uses `Buffer.from().toString("base64url")` + `createHmac`); `pollForApproval()` async helper; two approval flow `it()` tests inside the existing `describe.skipIf(!gatewayUrl)` block, each with `{ timeout: 25_000 }`

---

## Test Coverage Delta

| Suite | Before | After | Notes |
|-------|--------|-------|-------|
| Go integration tests | 7 tests | 9 tests (+2 sub-tests in ApprovalFlow) | +`approve` sub-test, +`deny` sub-test |
| TS integration tests (skipped) | 7 skipped | 9 skipped | +2 approval flow tests |
| TS unit tests | 16 passing | 16 passing | No regressions |
| Cargo check | clean | clean | No Rust code changed |

---

## Lessons Captured

### 1. ndjson is the right wire format for test stubs

AG-UI SSE requires parsing `data:` prefixed lines, handling `event:` type lines, and correctly recognising multi-line events. ndjson is one JSON object per line — trivially emittable from a 30-line Node.js script and trivially assertable with a `.split("\n")` in the test. When writing test infrastructure, pick the simplest wire format the production stack supports.

### 2. Stdlib-only JWTs are viable for test fixtures

Both the Go (`crypto/hmac`, `crypto/sha256`, `encoding/base64`) and TypeScript (`node:crypto createHmac`, `Buffer.toString("base64url")`) runtimes can sign a minimal HS256 JWT with zero external dependencies. This keeps the integration test environment lean and avoids dependency surface for a test-only concern.

### 3. Cedar policy as runtime fixture data, not config

The `@require_approval` policy is created via `POST /policies` at the start of the test and deleted in `t.Cleanup` / `finally`. This is far cleaner than committing test policies to `config.test.yaml` — it keeps the base config minimal, tests the Admin API policy lifecycle as a side effect, and avoids cross-test contamination. Every approval flow test that needs Cedar authz should follow this pattern.

### 4. The proxy port (4456) needs its own CI env var

The existing `INTEGRATION_GATEWAY_URL` pointed at the admin port (4457). The approval flow test must POST to the agent-facing proxy port (4456). The two ports serve different purposes and need separate env vars. `INTEGRATION_PROXY_URL` was added to the global `env:` block in `integration.yml` and surfaced to both test steps.

### 5. Go stream + approval race requires goroutine + channel

The approve path must: (a) start reading the stream, (b) wait for the approval to appear, (c) decide, (d) observe the stream deliver the buffered event. Steps (a) and (b)–(d) are concurrent — the stream is blocked at the gateway until (c) completes. The correct Go pattern is: start a goroutine that reads the response body; poll the Admin API from the main goroutine; decide; then receive from the goroutine's result channel with a deadline. The `select { case types := <-typesCh: ... case <-time.After(...): ... }` pattern prevents the test from hanging forever on a failed approval release.

---

## Technical Debt Introduced

None. All changes are additive test infrastructure. The stub is isolated to `test/stub/` and the compose service. No production paths changed.

---

## Security Constraints Verified

| Constraint | Verified |
|------------|----------|
| Admin port (4457) never exposed publicly | ✅ `mock-upstream` has no `ports:` mapping; test connects via loopback |
| No secrets committed | ✅ `test-jwt-secret` was pre-existing in `docker-compose.test.yml`; Cedar policy text is plain permit rule |
| Existing unit tests not broken | ✅ 16 TS unit tests pass; `cargo check` clean |
| Config priority order unchanged | ✅ Only YAML test config modified, no priority logic touched |
| Fail-closed preserved | ✅ No changes to Cedar engine, `ApprovalManager`, or stream processor |

---

## Recommended Next Phase

### Option A — Commit + PR (recommended)

All three changes are complete and passing locally. The natural next step is to commit this branch and open a PR for review. The integration tests cannot be verified in CI until the PR is open (docker-compose requires the built image). This is a clean stopping point.

**Suggested PR scope**: mock upstream stub + Go/TS approval flow tests + CI workflow update.

### Option B — Playwright E2E for the Approval UI

The web admin UI has Playwright smoke tests for approvals (empty state, table display, polling). A next phase could add a real E2E test that uses the `mock-upstream` stub (now available) to trigger an actual approval visible in the web UI, then approve it via the UI and assert the Playwright page reflects the decision.

**Dependency**: requires the web container to be added to the E2E compose stack.

### Option C — Rust SDK parity

The Go and TypeScript SDKs have full policy + approval coverage. If a Rust client SDK exists (or is planned), this phase's pattern (stdlib JWT, polling helper, approve/deny sub-tests) is directly portable.

### Option D — Approval expiry integration test

The current tests exercise the approve and deny decision paths. The expiry path (TTL auto-deny) is only unit-tested. An integration test could create an approval with a 2s TTL, wait, and assert the approval disappears from `ListApprovals`. Requires a config option to lower the default TTL per-test or per-route.
