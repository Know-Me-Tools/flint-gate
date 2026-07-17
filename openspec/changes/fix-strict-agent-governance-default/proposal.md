# fix-strict-agent-governance-default

**Phase:** beta-release-readiness / Phase 2 (Serious gap S-3)

## Problem

`strict_agent_governance: false` is the default. A route with a JWT auth
provider (agent-reachable) that has no `authorize` hook is silently proxied
without Cedar evaluation. Beta customers building agent pipelines expect the
gateway to protect by default.

## Solution

Change the default to `strict_agent_governance: true`. Any agent-reachable
route without an `authorize` hook will fail at startup (or reject on hot-reload).

Verify all test fixtures are compliant. Update `config.example.yaml` to
document that this is now the default and how to disable it for legacy routes.

## Files to change

- `crates/flint-gate-core/src/config/types.rs` — change default to `true`
- `config.example.yaml` — update comment
- `config.test.yaml` — verify compliance (routes have `authorize` hooks)
- Add unit test: strict mode + ungoverned agent route → startup error
