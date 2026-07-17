# Goals — agent-authz-budget-rate-limiting

_Seeded from `sdk-integration-tests/reflection.md` → "Recommended Next Phase → Option A"._

The active branch `feat/agent-authz-budget-rate-limiting` contains in-progress work on
per-tool authorization budgets and rate limiting for the agent gateway. This phase
closes out that feature track, wires it into the CI integration workflow, and adds SDK
client methods for the approval/budget endpoints.

## Goals

1. **Complete per-tool authorization budget enforcement** — ensure the Cedar policy
   engine enforces per-tool call budgets (token count, request rate, time window) at
   the flint-gate proxy layer. Any partially-implemented budget enforcement from the
   branch should be brought to a shippable state.

2. **Rate limiting for agent tool calls** — add or complete rate limiting middleware
   that enforces per-agent, per-tool, and per-scope call rate limits. Limits should
   be configurable via YAML (`config.yaml`) and per-route Cedar policy.

3. **SDK client methods for approval/budget endpoints** — add Go and TypeScript SDK
   methods for:
   - Submitting and polling approval requests
   - Querying budget status (remaining tokens, call counts)
   These are the methods deferred from `sdk-integration-tests` because they didn't
   exist yet.

4. **Integration tests for budget/approval endpoints** — extend `sdks/go/integration_test.go`
   and `sdks/typescript/src/__tests__/integration.test.ts` with test cases for the
   new SDK methods, running against the live `docker-compose.test.yml` fixture.

5. **CI wiring** — ensure `.github/workflows/integration.yml` covers the new SDK
   methods. No new workflow needed; extend the existing integration job.

## Success Criteria

- [ ] Budget enforcement is tested end-to-end (policy → proxy → SDK)
- [ ] Rate limiting is configurable and documented in `config.yaml`
- [ ] Go SDK has `SubmitApproval`, `GetApprovalStatus`, `GetBudgetStatus` (or equivalent)
- [ ] TypeScript SDK mirrors the Go SDK methods
- [ ] Integration tests for new methods pass against the live fixture
- [ ] `cargo test --workspace`, `go test ./...`, and `pnpm test` continue to pass
- [ ] No admin port exposed to public internet (security constraint preserved)

## Explicitly Out of Scope

- New Cedar policy authoring UI (deferred to a cedar UX phase)
- Load/performance testing of rate limiting (deferred to a dedicated load-test phase)
- Proxy-path SSE integration tests (deferred — requires upstream stub service)
- TS SDK idempotent delete/revoke hardening (small; can be bundled or done separately)

## Security Constraints (carry-forward — non-negotiable)

- Never expose admin server (port 4457) to public internet
- Never commit secrets, JWT signing keys, or production database credentials to version control
- Never break existing unit tests without updating or replacing them
- Fail-closed: no silent-allow path under any circumstance
