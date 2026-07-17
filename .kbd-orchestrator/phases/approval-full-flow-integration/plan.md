# Plan — approval-full-flow-integration

_Produced from: assessment.md_
_Backend: openspec_
_Changes: 3_

## Ordering Rationale

Change 1 (`add-mock-upstream-stub`) is a strict prerequisite: without a streaming
upstream that emits a tool-call ndjson sequence, the approval path in flint-gate is
never triggered. Changes 2 and 3 can run independently of each other once Change 1 is
in place, but both require the mock upstream to be present in the compose fixture.

## Changes

| Order | Change ID | Description | Agent | Depends On |
|-------|-----------|-------------|-------|------------|
| 1 | `add-mock-upstream-stub` | Node.js one-file stub server + docker-compose service + config route | general-purpose | — |
| 2 | `add-go-approval-flow-test` | `TestIntegration_ApprovalFlow` in `sdks/go/integration_test.go` | general-purpose | Change 1 |
| 3 | `add-ts-approval-flow-test` | TypeScript approval full-flow test in `integration.test.ts` | general-purpose | Change 1 |

## Change 1 — `add-mock-upstream-stub`

**What**: Add a Node.js HTTP stub server (`test/stub/server.mjs`) and wire it into
`docker-compose.test.yml` as a new `mock-upstream` service. Add a streaming route in
`config.test.yaml` that routes to the stub with `stream.enabled: true` and an
`Authorize` pre-request hook.

**Why**: The approval path fires only when the stream processor receives a tool-call
event from an upstream. The existing `/health` upstream returns JSON, not a streaming
tool-call event sequence.

**Stub behaviour**: POSTed to any path → responds with `Content-Type: application/x-ndjson`
containing a minimal 3-line ndjson sequence:
```
{"type":"TOOL_CALL_START","toolCallId":"tc-001","toolCallName":"integ_test_tool"}
{"type":"TOOL_CALL_ARGS","toolCallId":"tc-001","delta":"{\"x\":1}"}
{"type":"TOOL_CALL_END","toolCallId":"tc-001"}
```

**Files changed**:
- `test/stub/server.mjs` (new)
- `docker-compose.test.yml` (add `mock-upstream` service)
- `config.test.yaml` (add `/stream-test` route with stream + Authorize hook)

**Cedar policy**: Created at runtime by the integration test via `POST /policies` —
not committed to config. The policy text is:
```cedar
@require_approval("human review required")
permit(principal, action, resource == Route::"integ_test_tool");
```

## Change 2 — `add-go-approval-flow-test`

**What**: Add `TestIntegration_ApprovalFlow` to `sdks/go/integration_test.go`.

**Test sequence**:
1. Create Cedar `@require_approval` policy via `CreatePolicy` (cleanup via `t.Cleanup`)
2. Send `POST http://<proxy-port>/stream-test` with `Content-Type: application/json`
   and `Accept: application/x-ndjson`, JWT-authenticated
3. Read the SSE/ndjson response in a goroutine; expect a `gate:approval_request`-style
   event to appear within 5s
4. Poll `ListApprovals` in a loop (500ms interval, 10s timeout) until the approval
   for this tool call appears
5. Call `DecideApproval(id, "approve")` — verify no error
6. Assert the goroutine reading the proxy response sees the unblocked `TOOL_CALL_START`
   event propagated through (stream continues)
7. Run the deny path: repeat steps 2–4 with a fresh request, then `DecideApproval(id, "deny")`
   and assert the stream closes with an error event

**Obtaining a JWT**: The test fixture uses Hydra client-credentials. Test can use a
static test JWT signed with `FLINT_GATE_JWT_SECRET=test-jwt-secret` (HS256), or
call the Hydra token endpoint. Use a static JWT for simplicity — the `signing_key_secret`
is `test-jwt-secret` and `issuer` is `http://flint-gate:4456`.

## Change 3 — `add-ts-approval-flow-test`

**What**: Mirror of Change 2 in TypeScript.

**Test structure**: Inside `describe.skipIf(!gatewayUrl)` block in
`sdks/typescript/src/__tests__/integration.test.ts`.

**Uses**: `fetch` for the streaming proxy request (Node.js 18+ native fetch supports
streaming); SDK `listApprovals`, `getApproval`, `decideApproval`. Timeout via
`AbortController` with 10s signal.

## Security Constraints Carried Forward

- Admin port (4457) stays loopback-bound — tests connect via `INTEGRATION_GATEWAY_URL`
- No secrets committed — Cedar policy text is plain; stub is plain JS; JWT secret
  (`test-jwt-secret`) is already in `docker-compose.test.yml` and `config.test.yaml`
- Fail-closed preserved — approval with expired TTL or no decision → auto-deny
- No changes to production code, Cedar engine logic, or ApprovalManager

## Definition of Done

- [ ] `test/stub/server.mjs` exists and is ~30 lines
- [ ] `docker-compose.test.yml` contains `mock-upstream` service with healthcheck
- [ ] `config.test.yaml` has `/stream-test` route with `stream.enabled: true` and
  `Authorize` hook
- [ ] `TestIntegration_ApprovalFlow` passes against the live compose fixture
- [ ] TypeScript approval flow test passes
- [ ] `go test -v -tags=integration ./...` green (no regressions)
- [ ] `pnpm test` green (no regressions)
- [ ] CI `integration.yml` passes (docker-compose up --wait picks up new service automatically)
