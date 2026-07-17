# Proposal: add-integration-test-fixture

## Why

The Go and TypeScript SDKs have no integration tests against a real running
gateway. Before any SDK integration tests can be written, there must be a
self-contained docker-compose fixture that can be started in CI with a single
`docker compose up --wait` command. The existing `docker-compose.smoke.yml` is
optimized for Playwright UI tests (includes a `web` Vite container) and mounts
a smoke config that requires Hydra client-credentials for every request. A
leaner fixture that exposes a static admin token lets SDK integration tests
run without OAuth token flows.

## What Changes

- **Create** `docker-compose.test.yml` — postgres + hydra-migrate + hydra +
  hydra-seed + flint-gate only (no `web` container); adds
  `FLINT_GATE_ADMIN_TOKEN=integration-test-token` to the flint-gate service
- **Create** `config.test.yaml` — mirrors `config.smoke.yaml` structure with
  `admin_token: integration-test-token` and minimal site/route seed; no changes
  to existing `config.smoke.yaml`
