# Assessment — sdk-integration-tests

_Generated: 2026-07-09_

## Summary

The Go and TypeScript SDKs have solid unit-test suites but **zero integration
tests** against a real running gateway. The existing `docker-compose.smoke.yml`
provides a near-complete integration fixture (Postgres + Hydra + flint-gate) but
is oriented toward Playwright UI smoke tests. This phase adds a parallel
`docker-compose.test.yml` (leaner, no web container), Go integration tests
tagged `//go:build integration`, TypeScript integration tests gated on
`INTEGRATION_GATEWAY_URL`, and a new `integration.yml` CI workflow.

---

## Codebase Inventory

### SDKs present

| SDK        | Location               | Unit tests          | Integration tests |
|------------|------------------------|---------------------|-------------------|
| Go         | `sdks/go/`             | `client_test.go` ✅  | **None** ❌       |
| TypeScript | `sdks/typescript/src/` | `admin.test.ts`, `stream.test.ts` ✅ | **None** ❌ |
| Flutter    | `sdks/flutter/test/`   | `client_test.dart` ✅ | **None** ❌       |

Flutter is out of scope per `goals.md`.

### Go SDK surface area

From `sdks/go/client.go` — methods integration tests must cover:

| Method | HTTP | Priority |
|--------|------|----------|
| `GetHealth` | GET /health | HIGH — proves connectivity |
| `GetReady` | GET /ready | HIGH |
| `GetRoutes` | GET /routes | HIGH |
| `CreateRoute` | POST /routes | HIGH |
| `UpsertRoute` | PUT /routes/{id} | MED |
| `DeleteRoute` | DELETE /routes/{id} | HIGH (idempotence) |
| `ListAPIKeys` | GET /api-keys | MED |
| `CreateAPIKey` | POST /api-keys | MED |
| `DeleteAPIKey` | DELETE /api-keys/{id} | MED |
| `CacheStats` | GET /cache/stats | LOW |
| `InvalidateCache` | POST /cache/invalidate | LOW |

Middleware (`NewMiddleware`, `RequireScope`) is already well-unit-tested via
httptest; integration coverage is lower priority.

The SSE layer (`StreamSSE`) requires a live proxy port (`:4456`) and a real
upstream. Integration-testing this would require a routed upstream service —
deferred; unit tests (`TestStreamSSE_EndToEnd`) are thorough.

### TypeScript SDK surface area

From `sdks/typescript/src/admin.ts` — `FlintGateAdmin` methods mirror the Go
client. `sdks/typescript/src/types.ts` defines `PolicyRow`, `RouteConfig`,
`ApiKey`, `HealthStatus`, etc.

Current unit tests in `admin.test.ts` mock the underlying `adminRequest` spy.
No live-network path is tested.

`stream.test.ts` constructs synthetic `ReadableStream` objects — no live
proxy tested.

### Existing docker-compose fixture

`docker-compose.smoke.yml` already provides:
- `postgres:16-alpine` (pg_isready healthcheck)
- `hydra-migrate` → `hydra` (JWT access tokens via JWKS on `:4444`)
- `hydra-seed` (one-shot client: `flint-e2e-client` / `flint-e2e-secret`)
- `flint-gate` (built from source, ports `:4456` / `:4457`)
- `web` container (Vite dev server on `:5173`) — **not needed for SDK tests**

`config.smoke.yaml` wires Hydra JWT auth and exposes the admin API on `:4457`.

### GitHub Actions

Existing workflows: `ci.yml` (Rust + Docker build), `e2e-smoke.yml`
(Playwright E2E, added last phase). No workflow runs Go/TS SDK tests.

---

## Gaps Found

### Gap 1 — No `docker-compose.test.yml`

`docker-compose.smoke.yml` is the correct reference architecture but drags in
the `web` container. An SDK integration fixture should be slimmer:
- Drop the `web` service (not needed for API tests)
- Use a `config.test.yaml` that hard-wires a known API key (no Hydra dependency
  for admin tests) while still including Hydra for proxy-level auth tests
- Mount pre-seeded routes and policies so tests have predictable state

**Decision**: create `docker-compose.test.yml` with postgres + hydra-migrate +
hydra + hydra-seed + flint-gate only. Reuse Hydra from `smoke` unchanged.
Create `config.test.yaml` with a static admin API key for simplicity.

### Gap 2 — No Go integration tests

`sdks/go/client_test.go` only covers SSE parsing, the HTTP plumbing, and
middleware using `httptest.NewServer`. There are no `//go:build integration`
tests that hit a live `flint-gate` admin port.

Required: `sdks/go/integration_test.go` with build tag `//go:build integration`
covering the critical path: health, routes CRUD, and API keys CRUD.

### Gap 3 — No TypeScript integration tests

`sdks/typescript/src/__tests__/admin.test.ts` and `stream.test.ts` use a
`vi.fn()` spy to intercept `adminRequest`. No test exercises a real HTTP client
path.

Required: `sdks/typescript/src/__tests__/integration.test.ts` using `vitest`
with a skip guard when `INTEGRATION_GATEWAY_URL` is unset.

### Gap 4 — No `integration.yml` CI workflow

No GitHub Actions job brings up `docker-compose.test.yml` and runs
`go test -tags integration` + `pnpm test:integration`. The new job must:
- Run `docker compose -f docker-compose.test.yml up -d --wait`
- Run Go integration tests against `INTEGRATION_GATEWAY_URL=http://localhost:4457`
- Run TS integration tests against the same URL
- Tear down the stack on completion/failure (via `always()`)

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| flint-gate binary not build-cached in CI → slow startup | MED | Use `docker build` layer cache in the workflow |
| Hydra health-ready race condition | MED | Existing `start_period: 10s` + `retries: 20` on hydra healthcheck is proven by the smoke stack |
| Test isolation — routes created by test leaking across test runs | MED | Use unique ID prefix per test run (`integration-test-<timestamp>`) + cleanup in `t.Cleanup` |
| Go module version pin (go 1.22) | LOW | No new deps needed; only stdlib + existing `nhooyr.io/websocket` |
| TS integration test adds vitest `globalSetup` or `--poolOptions` complexity | LOW | Use `process.env.INTEGRATION_GATEWAY_URL` guard at describe-level; no custom setup file needed |

---

## Open Questions

1. **API key auth for admin in tests**: Should integration tests use a static
   `FLINT_GATE_ADMIN_TOKEN` env var, or test without auth (admin port is
   trusted-network-only per security constraints)? Recommendation: use a static
   token set in `docker-compose.test.yml` → `FLINT_GATE_ADMIN_TOKEN=integration-test-token`
   and propagate to test clients via env.

2. **Approval/budget endpoints**: `goals.md` mentions "approval submission and
   decision" and "budget check" but neither Go nor TS SDK has these client
   methods yet. Scope for this phase should be **admin CRUD only** (health,
   routes, api-keys). Approval/budget tests are deferred until the SDK methods
   exist.

3. **`config.test.yaml`**: Should it reuse `config.smoke.yaml` verbatim or be a
   separate file? Separate is cleaner — allows test-specific tweaks without
   risking smoke regressions.

---

## Recommended Changes (ordered)

| # | Change ID | Description | Why first |
|---|-----------|-------------|-----------|
| 1 | `add-integration-test-fixture` | `docker-compose.test.yml` + `config.test.yaml` | All other changes depend on a running fixture |
| 2 | `add-go-sdk-integration-tests` | `sdks/go/integration_test.go` with build tag | Go is the reference SDK; validate its client against live gateway |
| 3 | `add-ts-sdk-integration-tests` | `sdks/typescript/src/__tests__/integration.test.ts` | TypeScript SDK integration, mirrors Go coverage |
| 4 | `add-integration-ci-workflow` | `.github/workflows/integration.yml` | CI gate — gates the phase close |

---

## Out-of-Scope Confirmation

Per `goals.md`:
- Rust SDK integration tests — skipped (Go provides equivalent coverage)
- Performance/load testing — deferred
- Production environment testing — fixture is CI/local only
- Approval/budget SDK methods — no client methods exist yet; deferred
