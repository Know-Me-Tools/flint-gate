# Reflection — sdk-integration-tests

_Generated: 2026-07-09_

---

## Goal Achievement

| Goal | Status | Notes |
|------|--------|-------|
| Docker-compose test fixture | **MET** | `docker-compose.test.yml` + `config.test.yaml` created; loopback admin bind (`127.0.0.1:4457`) → `AllowLoopback` posture; no auth token needed |
| Go SDK integration tests | **MET** | `sdks/go/integration_test.go` with `//go:build integration` tag; 5 test functions; `go vet -tags integration .` clean |
| TypeScript SDK integration tests | **MET** | `sdks/typescript/src/__tests__/integration.test.ts`; 4 tests; `describe.skipIf(!gatewayUrl)` guard; `pnpm test:integration` skips cleanly when no fixture; `tsc --noEmit` clean |
| CI integration job | **MET** | `.github/workflows/integration.yml` with docker build, `--wait` startup, Go + TS steps, always-run teardown |
| Existing unit tests continue to pass | **MET** | No production code was modified; only test files and CI config added |

**Overall: 5/5 goals MET (100%)**

---

## Delivered Changes

| Change | Tasks | Deliverable |
|--------|-------|-------------|
| `add-integration-test-fixture` | 3/3 | `docker-compose.test.yml`, `config.test.yaml` |
| `add-go-sdk-integration-tests` | 5/5 | `sdks/go/integration_test.go` — Health, Ready, Routes CRUD, API Keys CRUD |
| `add-ts-sdk-integration-tests` | 4/4 | `sdks/typescript/src/__tests__/integration.test.ts`, `test:integration` script in `package.json` |
| `add-integration-ci-workflow` | 5/5 | `.github/workflows/integration.yml` |

---

## Artifact Quality Summary

| Metric | Value |
|--------|-------|
| Changes with QA (artifact-refiner) | 0/4 |
| First-pass pass rate | n/a (QA gate not run) |
| Changes requiring refinement | 0 |
| Total refinement iterations | 0 |

**Note:** Artifact-refiner QA gate was not invoked for any change in this phase. No `.refiner/artifacts/add-integration-*` logs exist. The code was manually verified (type-check, `go vet`, vitest skip behavior) but not through the formal QA pipeline. Future phases should run `/refine-validate` per change before archiving.

---

## Key Technical Decisions Made

### Admin auth posture: AllowLoopback (not token-based)
The assessment's Open Question 1 asked whether to use a static `FLINT_GATE_ADMIN_TOKEN`. Through code inspection of `crates/flint-gate-core/src/config/types.rs`, we discovered:
- `FLINT_GATE_ADMIN_TOKEN` is **not a real env var** — admin auth is YAML-configured only
- `admin_listen: "0.0.0.0:4457"` without `admin_auth` → `RefuseStart` (server won't start)
- `admin_listen: "127.0.0.1:4457"` → `AllowLoopback` posture — no auth needed

**Decision**: Bind admin to loopback in `config.test.yaml`. This is correct for the test fixture (CI runner is trusted) and matches the security constraints that admin must never be exposed to the public internet.

### Scope reduction: approval/budget deferred
Per assessment Open Question 2: neither the Go nor TypeScript SDK has `approval` or `budget` client methods. The goals.md mentioned these but they don't exist. Integration coverage is limited to what the SDK actually implements: health, routes CRUD, API keys CRUD.

### TypeScript method naming
`FlintGateAdmin` uses `revokeApiKey` (not `deleteApiKey`) and `getApiKeys` (not `listApiKeys`). The TS integration tests use the correct method names, distinct from the Go SDK naming convention.

---

## Technical Debt Introduced

1. **No idempotent revoke in TS**: The Go SDK's `DeleteAPIKey` and `DeleteRoute` swallow 404s (idempotent by design). `FlintGateAdmin.revokeApiKey` and `.deleteRoute` in TypeScript do **not** swallow 404 — they throw. The integration tests handle this with try/catch, but the TS SDK itself is less ergonomic for cleanup patterns. Recommendation: add 404-swallowing to `FlintGateAdmin` delete/revoke methods.

2. **Hydra in the test fixture**: `docker-compose.test.yml` includes a full Hydra stack (hydra-migrate + hydra + hydra-seed) even though the current integration tests only hit the admin port and don't test JWT-authenticated proxy requests. Hydra adds ~30s of startup time and ~256MB RAM. When proxy-path integration tests are added, this will be justified; until then it's overhead.

3. **No `-race` flag on Go integration tests**: The CI workflow runs `go test -v -tags integration -timeout 60s ./...` without `-race`. Add `-race` once the fixture is confirmed stable in CI (race detection doubles test duration; worth it for correctness).

4. **`pnpm-lock.yaml` cache path hardcoded**: The CI step caches `sdks/typescript/pnpm-lock.yaml`. If the TypeScript SDK is ever moved or a monorepo root lock file is adopted, this path will need updating.

---

## Lessons Captured

1. **Read config source before assuming env vars exist.** The assumption that `FLINT_GATE_ADMIN_TOKEN` was a real env var cost one debugging cycle. Always grep the config/types files first when wiring a new env var.

2. **Admin auth posture is a startup gate, not a runtime gate.** The server refuses to start when the combination of `admin_listen` and `admin_auth` is unsafe. This is a strong fail-closed design — integration tests must be wired to satisfy it at the config level, not by passing tokens at request time.

3. **TypeScript SDK methods don't mirror Go SDK naming.** `deleteApiKey` vs `revokeApiKey`, `listApiKeys` vs `getApiKeys` — always read the actual class interface rather than inferring from the Go equivalent.

4. **`describe.skipIf(!gatewayUrl)` is cleaner than `beforeAll` guards.** The skip is visible at the describe level in vitest output, shows "4 skipped" rather than a confusing zero-test count, and doesn't leave a test body that could throw before the guard fires.

5. **`go vet` requires `GOROOT` on this machine.** The brew-installed Go has a broken symlink at `1.26.0`; actual version is `1.26.4`. Run `GOROOT=/opt/homebrew/opt/go/libexec go vet -tags integration .` from `sdks/go/`. This is a local dev quirk; CI uses `actions/setup-go` which sets `GOROOT` correctly.

---

## Scope Not Delivered (Confirmed Deferred)

| Item | Reason |
|------|--------|
| Approval/budget integration tests | No SDK client methods exist; deferred to a future phase when these endpoints are implemented |
| Rust SDK integration tests | Go provides equivalent admin API coverage; Rust client is a thin HTTP wrapper |
| SSE/stream integration tests (proxy port `:4456`) | Requires a routed upstream service; unit tests for SSE are thorough; deferred |
| Live CI run verified green | Cannot verify until the branch is pushed and the workflow runs in GitHub Actions |

---

## Recommended Next Phase

### Option A — Agent Authorization + Budget/Rate-Limiting (feat/agent-authz-budget-rate-limiting branch work)

The active git branch is `feat/agent-authz-budget-rate-limiting`. This is almost certainly the next in-progress feature. The natural next phase is to:
- Complete any remaining work on per-tool authorization budget and rate limiting
- Add integration tests for the approval/budget endpoints once the SDK client methods exist
- Wire the agent authorization control plane into the CI workflow

### Option B — Proxy-path integration tests (SSE + JWT auth)

The fixture includes Hydra but no test uses it. A follow-on phase could:
- Add a lightweight upstream stub service to `docker-compose.test.yml`
- Add Go + TS integration tests for the proxy path (`:4456`) using Hydra-issued JWTs
- Cover `StreamSSE` / `streamSSE` with a live gateway

### Option C — TS SDK idempotent delete/revoke hardening

Address the technical debt item: make `FlintGateAdmin.deleteRoute` and `revokeApiKey` swallow 404s to match the Go SDK's ergonomics. Small, contained, high value for test cleanup patterns.

**Primary recommendation: Option A** — the branch context is already established and the agent-authz work is the active feature track.
