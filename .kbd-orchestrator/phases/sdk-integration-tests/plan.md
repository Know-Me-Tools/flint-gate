# Plan — sdk-integration-tests

_Generated from assessment.md — 2026-07-09_

## Backend: OpenSpec

## Ordering Rationale

Change 1 must land first — the fixture is a hard dependency for changes 2 and 3.
Changes 2 and 3 (Go and TS tests) are logically parallel but executed sequentially
to keep the context clean. Change 4 wires CI and is the phase-close gate.

## Changes

| # | Change ID | Description | Blocks |
|---|-----------|-------------|--------|
| 1 | `add-integration-test-fixture` | `docker-compose.test.yml` + `config.test.yaml` lightweight SDK test stack | changes 2, 3, 4 |
| 2 | `add-go-sdk-integration-tests` | `sdks/go/integration_test.go` with `//go:build integration` tag | change 4 |
| 3 | `add-ts-sdk-integration-tests` | `sdks/typescript/src/__tests__/integration.test.ts` + vitest config update | change 4 |
| 4 | `add-integration-ci-workflow` | `.github/workflows/integration.yml` CI job that boots the fixture and runs both SDK suites | phase close |

## Per-Change Detail

### Change 1 — add-integration-test-fixture

**Goal**: Provide a lean `docker-compose.test.yml` (postgres + Hydra + flint-gate,
no web container) and a `config.test.yaml` that hard-wires a static admin token
so integration tests can authenticate without Hydra client-credentials.

**Tasks**:
1. Create `docker-compose.test.yml` based on `docker-compose.smoke.yml`, removing
   the `web` service; add `FLINT_GATE_ADMIN_TOKEN=integration-test-token` env
2. Create `config.test.yaml` (mirrors `config.smoke.yaml`) with a static
   `admin_token: integration-test-token` field and test-oriented site/route seed
3. Verify `docker compose -f docker-compose.test.yml up -d --wait` succeeds locally

**Spec**: `openspec/changes/add-integration-test-fixture/`

---

### Change 2 — add-go-sdk-integration-tests

**Goal**: Add `sdks/go/integration_test.go` exercising `GetHealth`, `GetReady`,
routes CRUD (`CreateRoute`, `GetRoutes`, `GetRoute`, `DeleteRoute`), and API key
CRUD (`CreateAPIKey`, `ListAPIKeys`, `DeleteAPIKey`) against the live fixture.

**Tasks**:
1. Create `sdks/go/integration_test.go` with `//go:build integration` tag
2. Implement `TestIntegration_Health` — verify status "ok"
3. Implement `TestIntegration_Routes` — create, list, get, delete; assert round-trip
4. Implement `TestIntegration_APIKeys` — create, list, delete; verify secret only returned once
5. Ensure `go test -tags integration ./...` passes against the running fixture

**Env**: `INTEGRATION_GATEWAY_URL` (default `http://localhost:4457`)

**Spec**: `openspec/changes/add-go-sdk-integration-tests/`

---

### Change 3 — add-ts-sdk-integration-tests

**Goal**: Add `sdks/typescript/src/__tests__/integration.test.ts` exercising
`FlintGateAdmin` methods (health, routes CRUD, API keys CRUD) against the live
fixture; tests skip when `INTEGRATION_GATEWAY_URL` is unset.

**Tasks**:
1. Create `sdks/typescript/src/__tests__/integration.test.ts`
2. Add `test:integration` script to `sdks/typescript/package.json` running
   `vitest run --reporter=verbose` with pattern `integration.test`
3. Implement `describe("FlintGateAdmin integration", ...)` with beforeAll skip guard
4. Implement health, route CRUD, and API key CRUD test cases
5. Verify `INTEGRATION_GATEWAY_URL=http://localhost:4457 pnpm test:integration` passes

**Spec**: `openspec/changes/add-ts-sdk-integration-tests/`

---

### Change 4 — add-integration-ci-workflow

**Goal**: Add `.github/workflows/integration.yml` that brings up
`docker-compose.test.yml`, waits for health, runs both SDK suites, and tears down.

**Tasks**:
1. Create `.github/workflows/integration.yml`
2. Steps: checkout → rust-toolchain → build Docker image → docker compose up --wait
3. Step: Go integration tests (`go test -race -tags integration ./sdks/go/...`)
4. Step: TypeScript integration tests (`pnpm --filter @know-me/flint-gate test:integration`)
5. Step: docker compose down (always)

**Triggers**: push/PR to `main` and `claude/flint-gate-auth-proxy-zQBD4`

**Spec**: `openspec/changes/add-integration-ci-workflow/`

---

## Constraints Applied

- No secrets in `docker-compose.test.yml` or `config.test.yaml` — admin token is
  a known test constant (`integration-test-token`), not a production credential
- Admin port (4457) is exposed only within the docker-compose network + host loopback
- Existing unit tests (`cargo test`, `go test ./...`, `pnpm test`) must not be
  broken by any change in this phase
- `//go:build integration` ensures the new Go tests are excluded from unit-only runs
- `INTEGRATION_GATEWAY_URL` guard ensures TS integration tests skip in unit-only CI
