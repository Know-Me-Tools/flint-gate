# Proposal: add-integration-ci-workflow

## Why

No GitHub Actions job currently verifies that the Go or TypeScript SDK
integration tests pass on CI. Without a CI gate, regressions in the admin API
serialization or auth handling can merge undetected. The `integration.yml`
workflow closes this gap by booting the `docker-compose.test.yml` fixture and
running both SDK integration suites on every push to the active branches.

## What Changes

- **Create** `.github/workflows/integration.yml` with:
  - Triggers: push/PR to `main` and `claude/flint-gate-auth-proxy-zQBD4`
  - Steps: checkout → Rust toolchain → build Docker image → `docker compose -f docker-compose.test.yml up -d --wait`
  - Go integration step: `go test -race -tags integration ./sdks/go/...`
  - TypeScript integration step: `pnpm --filter @know-me/flint-gate test:integration`
  - Teardown step: `docker compose -f docker-compose.test.yml down` (always runs)
