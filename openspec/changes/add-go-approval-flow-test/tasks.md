# Tasks — add-go-approval-flow-test

- [ ] Add `testJWT` helper to `sdks/go/integration_test.go` (HS256, stdlib only)
- [ ] Add `INTEGRATION_PROXY_URL` env-var helper (default http://127.0.0.1:4456)
- [ ] Implement `TestIntegration_ApprovalFlow` — approve path (create policy → POST /stream-test → poll approvals → approve → assert TOOL_CALL_START received)
- [ ] Implement deny path inside same test (fresh request → poll → deny → assert stream closes)
- [ ] Ensure `t.Cleanup` deletes Cedar policy after test (not idempotent — only delete once)
- [ ] Run `go test -v -tags=integration -run TestIntegration_ApprovalFlow ./sdks/go/... 2>&1 | tail -20` and confirm PASS
