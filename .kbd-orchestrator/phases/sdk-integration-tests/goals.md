# Goals — sdk-integration-tests

_Seeded from `e2e-coverage-and-ui-smoke-tests/reflection.md` → "Recommended Next Phase → Option A"._

The Go, Rust, and TypeScript SDKs were built in previous phases but have no integration
tests against a real running gateway. Unit tests pass but behavior against the actual
gateway is untested. This phase closes that gap by adding a docker-compose fixture and
SDK integration test suites.

1. **Docker-compose test fixture** — create `docker-compose.test.yml` that brings up:
   - The flint-gate binary (built from source or a local image)
   - A mock/stub Hydra instance (or a lightweight JWT issuer) for token generation
   - Required environment wiring (admin port, proxy port, test API key)
   The fixture must be self-contained and start/stop cleanly in CI.

2. **Go SDK integration tests** — add `_test.go` files under `sdk/go/` (or `clients/go/`)
   that exercise the primary SDK methods against the live fixture:
   - Route listing and creation
   - Policy evaluation (permit/deny)
   - Approval submission and decision
   - Budget check
   Tests must be tagged `//go:build integration` and skipped in unit-only runs.

3. **TypeScript SDK integration tests** — add integration tests under `sdk/typescript/`
   (or `clients/typescript/`) covering the same surface as the Go tests.
   Tests must be skipped when `INTEGRATION_GATEWAY_URL` is unset.

4. **CI integration job** — add `.github/workflows/integration.yml` that runs the
   docker-compose fixture and executes all SDK integration test suites.
   The job must pass on the active branch before the phase closes.

## Success Criteria

- [ ] `docker-compose.test.yml` starts the full stack cleanly (`docker compose -f docker-compose.test.yml up -d --wait`)
- [ ] Go integration tests pass against the live fixture
- [ ] TypeScript integration tests pass against the live fixture
- [ ] `integration.yml` CI job passes on the branch
- [ ] Existing unit tests (`cargo test`, `go test ./...`, `pnpm test`) continue to pass

## Explicitly Out of Scope

- Rust SDK integration tests (the Rust client is an HTTP wrapper; Go tests provide equivalent coverage)
- Performance/load testing (defer to a dedicated load-test phase)
- Production environment testing (fixture is local/CI only)
- New gateway features (integration test only, no feature development)
