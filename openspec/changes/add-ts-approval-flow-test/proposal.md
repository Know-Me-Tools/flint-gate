# add-ts-approval-flow-test

## Summary

Add an approval full-flow integration test to
`sdks/typescript/src/__tests__/integration.test.ts`, mirroring the Go test.

Exercises the complete cycle using the TypeScript SDK methods and native Node.js `fetch`
for the streaming proxy request:

> agent request → Cedar `@require_approval` policy → approval buffered → admin decides
> → stream unblocked (approve) or closed (deny)

## Motivation

The TypeScript SDK has `listApprovals`, `getApproval`, and `decideApproval`. The approval
smoke test only verifies `listApprovals` returns an array. This change adds a real
round-trip test.

## Scope

### Files

| File | Action | Description |
|------|--------|-------------|
| `sdks/typescript/src/__tests__/integration.test.ts` | Edit | Add approval flow test inside `describe.skipIf(!gatewayUrl)` block |

### Test Structure

```typescript
it('approval full-flow: approve path', async () => {
  // 1. Create @require_approval Cedar policy
  const policy = await adminClient.createPolicy({
    id: `approval-integ-${Date.now()}`,
    text: `@require_approval("human review required")\npermit(principal, action, resource == Route::"integ_test_tool");`
  })
  try {
    // 2. POST to proxy /stream-test with streaming accept header + JWT
    const ac = new AbortController()
    setTimeout(() => ac.abort(), 15_000)
    const resp = fetch(`${proxyUrl}/stream-test`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${testJWT()}`, Accept: 'application/x-ndjson' },
      signal: ac.signal,
    })
    // 3. Poll ListApprovals until approval appears (10s)
    const approvalId = await pollForApproval(adminClient, 10_000)
    // 4. Approve
    await adminClient.decideApproval(approvalId, 'approve')
    // 5. Read ndjson lines from resp, assert TOOL_CALL_START present
    const body = await (await resp).text()
    expect(body).toContain('TOOL_CALL_START')
  } finally {
    await adminClient.deletePolicy(policy.id)
  }
}, 20_000)

it('approval full-flow: deny path', async () => {
  // Similar, but decideApproval(id, 'deny') and assert stream closed/errored
}, 20_000)
```

### JWT Generation

Inline `testJWT()` function using Node.js `crypto` module (no external dep):
```typescript
import { createHmac } from 'node:crypto'
function testJWT(): string { /* HS256, test-jwt-secret */ }
```

### Proxy URL

Read from `process.env.INTEGRATION_PROXY_URL`, default `http://127.0.0.1:4456`.

### Test Timeouts

Vitest default timeout is 5s — approval tests set `{ timeout: 20_000 }` in the `it` options.

## Acceptance Criteria

- [ ] Both approval flow tests pass against the live compose fixture
- [ ] Tests are skipped (not failed) when `INTEGRATION_GATEWAY_URL` is not set
- [ ] Cedar policy cleaned up in `finally` block (not left in DB)
- [ ] All 16 unit tests and existing 7 integration tests continue to pass
- [ ] `pnpm test` exits 0

## Security Constraints

- `testJWT` uses `test-jwt-secret` — already in `docker-compose.test.yml`, not new
- Cedar policy created at runtime, not committed to config
