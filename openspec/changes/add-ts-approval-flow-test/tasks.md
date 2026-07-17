# Tasks — add-ts-approval-flow-test

- [ ] Add `testJWT()` helper using Node.js `node:crypto` (HS256, test-jwt-secret, stdlib only)
- [ ] Add `proxyUrl` constant from `process.env.INTEGRATION_PROXY_URL` (default http://127.0.0.1:4456)
- [ ] Add `pollForApproval(client, timeoutMs)` helper that retries `listApprovals` until `length > 0`
- [ ] Implement approve-path test (createPolicy → POST /stream-test → pollForApproval → decideApproval approve → assert TOOL_CALL_START in body)
- [ ] Implement deny-path test (same but decideApproval deny → assert stream closed)
- [ ] Set `timeout: 20_000` on both tests via `it` options
- [ ] Run `pnpm test 2>&1 | tail -20` and confirm all tests pass
