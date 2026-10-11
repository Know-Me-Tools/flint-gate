---
name: gate-control-plane
description: "Maintain Gate admin authorization, configuration precedence, durable storage, cache and replicated authority readiness."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-control-plane

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Own crates/flint-gate-core/src/admin/**, config/**, cache/**, db/**, authority/** and crates/flint-gate-core/migrations/**. Read config loader and database migrations instead of copying older skill examples. Preserve CLI > environment > YAML precedence and separate proxy/admin listeners.

Admin operations require authenticated administrative authority, safe secret handling and auditable changes. Never expose admin 4457 publicly as a setup shortcut. Redact secrets in API responses, snapshots, errors and UI. Coordinate any new admin contract with console/SDK owners.

Specify atomic publication, revision checks and rollback for route/config/policy updates. Validate before replacing a working configuration. Distinguish cache TTL from authoritative revocation; test Redis failure, stale snapshots and worker races. Trace AuthorityConsumer::synchronize -> catch_up -> replay and readiness transitions; define ordering, replay idempotency, cursor persistence and fencing after restarts.

Coordinate policies with gate-policy and budgets with gate-streams. Prove stale authorization does not silently become fresh because a cache is reachable. Include migration/recovery tests where storage semantics change; do not claim exactly-once event delivery from a cursor alone.

## Primary references

- [cedarSecurity](https://docs.cedarpolicy.com/other/security.html)
