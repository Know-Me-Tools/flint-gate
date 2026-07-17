# Goals — sdk-integration-test-expansion

_Seeded from `agent-authz-budget-rate-limiting/reflection.md` → "Recommended Next Phase → Option A"._

The integration test scaffold from the `sdk-integration-tests` phase covers basic
route and API key creation. This phase fills out the remaining admin API surface
that is testable against the live `docker-compose.test.yml` fixture — no new
infrastructure required.

## Goals

1. **Policy CRUD integration tests** — extend `sdks/go/integration_test.go` and
   `sdks/typescript/src/__tests__/integration.test.ts` with test cases for:
   - `createPolicy` / `updatePolicy` / `deletePolicy`
   - `getPolicyHistory` (verify version rows are created)
   - `rollbackPolicy` (verify the rollback row appears in history)

2. **Route lifecycle integration tests** — add coverage for the full route
   lifecycle (update + delete) that the current tests don't exercise:
   - `updateRoute` — modify an existing route and verify the change
   - `deleteRoute` — delete a route and verify 404 on subsequent get

3. **API key lifecycle integration tests** — extend the API key tests to cover
   the complete lifecycle:
   - `createApiKey` with explicit key value
   - `getApiKeys` — verify the new key appears
   - `revokeApiKey` — verify revocation (key present but `revokedAt` set, or 404)

4. **Approval list smoke test** — add a trivial test that calls `listApprovals()`
   and asserts the response is an empty array when no approvals are pending.
   This verifies the endpoint is reachable and the SDK normalizer doesn't crash.

5. **CI green** — ensure `.github/workflows/integration.yml` continues to pass
   with all new tests against the live fixture.

## Success Criteria

- [ ] Go integration tests cover all 5 areas above and pass against the live fixture
- [ ] TypeScript integration tests mirror Go coverage and pass
- [ ] `cargo test --workspace`, `go test ./...`, and `pnpm test` continue to pass
- [ ] No new docker-compose services required (existing fixture is sufficient)
- [ ] Security constraints preserved (admin port loopback-only, no secrets committed)

## Explicitly Out of Scope

- Full approval-flow integration tests (require live stream + upstream stub — Option B)
- Admin UI enhancements (Option C — separate phase)
- Load/performance testing of any endpoints
- New SDK methods beyond what already exists

## Security Constraints (carry-forward — non-negotiable)

- Never expose admin server (port 4457) to public internet
- Never commit secrets, JWT signing keys, or production database credentials to version control
- Never break existing unit tests without updating or replacing them
- Fail-closed: no silent-allow path under any circumstance
