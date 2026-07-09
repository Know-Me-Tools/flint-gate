# Goals — approval-full-flow-integration

_Seeded from `sdk-integration-test-expansion/reflection.md` → "Recommended Next Phase → Option A"._

The approval smoke test added in the previous phase confirms the `listApprovals` endpoint is
reachable. This phase closes the last major integration gap: a full end-to-end proof that the
human-in-the-loop tool-call gating flow works — from an agent-facing streaming request, through
the approval buffer, to an admin decision, and back to the unblocked response.

## Goals

1. **Mock upstream service** — add a lightweight stub server to `docker-compose.test.yml`
   that can emit a streaming tool-call response (SSE or chunked JSON). The stub must be
   scriptable so tests can trigger an approval-required tool call on demand.

2. **Approval full-flow integration test (Go)** — extend `sdks/go/integration_test.go`
   with a test that:
   - Sends a request through the agent-facing proxy port that triggers an approval
   - Asserts the approval appears in `ListApprovals`
   - Calls `DecideApproval` with `approve` or `deny`
   - Asserts the buffered response is unblocked (approve) or rejected (deny)

3. **Approval full-flow integration test (TypeScript)** — mirror the Go test in
   `sdks/typescript/src/__tests__/integration.test.ts`.

4. **CI green** — `docker-compose.test.yml` with the stub upstream starts cleanly and
   all new approval flow tests pass in CI (`.github/workflows/integration.yml`).

## Success Criteria

- [ ] Mock upstream service added to `docker-compose.test.yml` and starts reliably
- [ ] Go approval full-flow test passes against the live fixture
- [ ] TypeScript approval full-flow test mirrors Go coverage and passes
- [ ] Existing `go test ./...` and `pnpm test` continue to pass (no regressions)
- [ ] CI workflow updated if docker-compose changes require it
- [ ] Security constraints preserved (admin port loopback-only, no secrets committed)

## Explicitly Out of Scope

- Load / stress testing of the approval buffer
- Admin UI E2E tests for the approval flow (separate Option C phase)
- Rust SDK parity (Option B — separate phase if Rust SDK exists)
- Changes to authorization logic or Cedar policies

## Security Constraints (carry-forward — non-negotiable)

- Never expose admin server (port 4457) to public internet
- Never commit secrets, JWT signing keys, or production database credentials to version control
- Never break existing unit tests without updating or replacing them
- Never change configuration priority order (CLI > env > YAML) without updating tests and docs
- Support any identity management system that has a pathway to generate JWTs (Ory Kratos/Hydra are reference, NEVER an IdP)
- Fail-closed: no silent-allow path under any circumstance
