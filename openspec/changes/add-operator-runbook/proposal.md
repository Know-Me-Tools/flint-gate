# add-operator-runbook

**Phase:** beta-release-readiness / Phase 3 (Medium gap M-4)

## Problem

There is no operator runbook. Beta customers deploying flint-gate to production
need guidance on:
- How to rotate JWT signing keys without downtime
- How to recover from a bad Cedar policy that blocks all traffic
- What the approval TTL janitor does and how to tune it
- How to interpret audit trail records
- How to write and validate Cedar policies for flint-gate's entity model

Without a runbook, beta customers will open support tickets for routine
operational tasks.

## Solution

Create two new documentation pages:

### `docs/docs/operations.md`

Cover:
1. Key rotation — update `auth.jwt_secret` in config + rolling restart
2. Policy recovery — `FLINT_GATE_BYPASS_AUTHZ=true` env var (if supported) or
   direct DB delete of blocking policy via admin API
3. Approval janitor — tuning `ttl_seconds` and `janitor_interval_seconds`
4. Audit trail — schema reference, common query patterns
5. Monitoring checklist — what metrics / log lines indicate a healthy gateway

### `docs/docs/cedar-policies.md`

Cover:
1. Entity model reference (User, Agent, Service, Route)
2. `@require_approval` annotation semantics
3. Common policy patterns (allow-all, agent-restricted, tool-specific)
4. Validating policies before upload (cedar-policy CLI or the admin API
   validate endpoint)
5. Policy debugging (enable `RUST_LOG=authz=debug`)

## Files to change

- `docs/docs/operations.md` (new)
- `docs/docs/cedar-policies.md` (new)
- `docs/docusaurus.config.ts` or sidebar config — add both pages to nav
