# add-oauth-e2e-ory

**Phase:** agent-gateway-exposure-operability · **Goal:** G4 (build-004)
**Scope:** `docker-compose.smoke.yml`, `web/e2e/*.spec.ts`,
`web/playwright.config.ts` (if needed), CI docs. **Depends on** changes 1–3.

## Why

The authenticated exposure surface (`/oauth/token`, `/oauth/introspect`,
Hydra-delegate) is only unit-tested against mocks. Prove it end-to-end against a
**real Ory stack** (Hydra/Kratos — the standing reference), including the
fail-closed denials the prior phases built.

## What

Extend the existing smoke harness (do not rebuild):
1. Add Ory **Hydra** (and **Kratos** where a session path is exercised) services
   to `docker-compose.smoke.yml`, seeded with a test OAuth client + JWKS.
2. Add Playwright E2E specs covering:
   - happy-path authenticated `/oauth/token` (client-credentials + RFC 8693
     exchange) and `/oauth/introspect`;
   - Hydra-delegate exchange returning a Hydra-minted token;
   - fail-closed denials: unauthenticated introspect → 401; over-rate → 429;
     Hydra transport error / non-2xx / redirect → deny; present `actor_token` →
     400.
3. Document the CI invocation (compose up → wait-healthy → run specs → teardown).

## Non-goals

- New test framework (Playwright is the harness).
- Load/perf testing (functional E2E only).

## Verification

The E2E suite passes against the composed Ory stack locally; specs use
deterministic waits (no flaky timeouts); documented CI entrypoint.
