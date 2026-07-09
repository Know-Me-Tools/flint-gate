# add-go-approval-flow-test

## Summary

Add `TestIntegration_ApprovalFlow` to `sdks/go/integration_test.go`, exercising the
complete human-in-the-loop approval cycle against the live compose fixture:

> agent request → Cedar `@require_approval` policy → approval buffered → admin decides
> → stream unblocked (approve) or closed (deny)

## Motivation

`TestIntegration_ApprovalSmoke` only verifies `ListApprovals` returns without error on
an idle fixture. The actual round-trip — trigger → poll → decide → observe outcome —
has no integration coverage. This change closes that gap for the Go SDK.

## Scope

### Files

| File | Action | Description |
|------|--------|-------------|
| `sdks/go/integration_test.go` | Edit | Add `TestIntegration_ApprovalFlow` function |

### Test Flow (Approve Path)

```
1. Create Cedar policy via c.CreatePolicy (text: @require_approval permit for integ_test_tool)
   → t.Cleanup: DeletePolicy
2. Build a minimal JWT (HS256, secret="test-jwt-secret", iss="http://flint-gate:4456")
   using a zero-dep approach: base64url(header) + "." + base64url(payload) + "." + hmac
3. POST http://<proxyURL>/stream-test
   → Authorization: Bearer <jwt>
   → Accept: application/x-ndjson
   → Content-Type: application/json, body: {}
   Start reading response in a goroutine
4. Poll ListApprovals (500ms interval, 10s timeout) until len > 0
5. Capture approval ID
6. DecideApproval(ctx, id, "approve") → assert no error
7. Goroutine: read ndjson lines from response body; assert TOOL_CALL_START received
8. Assert goroutine completes within 5s
```

### Test Flow (Deny Path)

```
Same steps 1–5 (reuse policy from cleanup-registered step; use a fresh HTTP request)
6. DecideApproval(ctx, id, "deny") → assert no error
7. Assert the response body closes (EOF or connection closed)
```

### JWT Generation

Rather than importing a JWT library, generate a static test JWT inline:

```go
// HS256 JWT signed with "test-jwt-secret"
// Claims: iss=http://flint-gate:4456, sub=integ-test, iat=now, exp=now+5min
func testJWT(t *testing.T) string { ... }
```

Uses only `crypto/hmac`, `crypto/sha256`, `encoding/base64`, `encoding/json` — all stdlib.

### Proxy URL

Read from env `INTEGRATION_PROXY_URL`, default `http://127.0.0.1:4456`.

## Acceptance Criteria

- [ ] `TestIntegration_ApprovalFlow` passes when the compose fixture is running
- [ ] Test cleans up the Cedar policy after completion (even on failure)
- [ ] Test does not leave stale approvals (TTL auto-expires within 300s)
- [ ] Test skips cleanly when `INTEGRATION_GATEWAY_URL` is not set
- [ ] All existing integration tests continue to pass (no regressions)
- [ ] `go test -v -tags=integration -run TestIntegration_ApprovalFlow ./...` exits 0

## Security Constraints

- JWT secret (`test-jwt-secret`) is already in `docker-compose.test.yml` — not a new
  committed secret
- Cedar policy text is created at runtime, not committed to config
- No admin port (4457) exposed beyond loopback — test uses standard `INTEGRATION_GATEWAY_URL`
