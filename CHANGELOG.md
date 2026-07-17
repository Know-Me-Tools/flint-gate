# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0-beta] - 2026-07-09

### Added

- **Cedar authorization engine** — embedded Cedar policy engine with hot-reload
  and write-time validation; policies load from database and are validated before
  being applied; invalid policies are rejected at write time (fail-closed)
- **Per-tool-call authorization** — streaming proxy intercepts ndjson tool calls
  and evaluates them against Cedar policies before forwarding to the client;
  unauthorized calls are dropped with an explicit error event
- **Human-in-the-loop approval flows** — `@require_approval` Cedar policy
  annotation suspends the stream at the tool call boundary and waits for an
  operator decision via the admin API; approved calls are forwarded, denied calls
  close the stream
- **Approval expiry / janitor** — pending approvals that exceed the configured
  TTL (`approval.ttl_seconds`) are automatically denied by a background janitor;
  the stream is closed and the approval swept from the pending table
- **Cedar entity schema** — built-in entity schema for `User`, `Agent`,
  `Service`, and `Route` entity types plus the `call_tool` action; policy
  annotations are validated at write time against the schema
- **Agent authorization control plane** — web admin UI for managing Cedar
  policies, viewing the authorization audit trail, and issuing approval decisions
- **Authorization audit trail** — every authorization decision (permit / forbid /
  pending) is recorded in the `authz_audit` table with principal, action,
  resource, matched policy, and timestamp
- **Budget and rate limiting** — per-identity token and request-rate budgets
  enforced at the proxy layer; in-process governor for burst protection;
  Redis-backed shared counters for multi-replica deployments
- **Multi-replica rate-limit warning** — startup warning when rate limiting is
  enabled without a Redis backend in a Kubernetes environment, where per-replica
  counters will not be shared
- **Strict agent governance** — `server.strict_agent_governance` option refuses
  to start when any route carries a tool call without a governing Cedar policy
- **Schema migrations** — versioned sqlx migrations for all database tables;
  applied automatically at startup via `sqlx::migrate!()`
- **ndjson streaming proxy** — transparent streaming proxy for ndjson tool-call
  streams with per-event authorization; WebSocket proxy with equivalent authz
- **Admin UI** — React-based administration interface for routes, policies,
  approvals, API keys, and audit logs; read-only mode indicator for pages that
  cannot modify state
- **Go SDK** (`sdks/go`) — typed Go client for the flint-gate admin API with
  integration tests
- **TypeScript SDK** (`sdks/typescript`) — typed TypeScript/Node.js client for
  the flint-gate admin API with integration tests
- **Playwright E2E scaffold** — end-to-end test scaffold using docker-compose
  and Playwright for smoke-testing the full stack
- **TLS support** — optional TLS on the proxy listener with `tls.fail_open`
  option for graceful degradation
- **JWT minting** — optional JWT minter backed by a database-sourced or
  config-sourced signing key; supports HS256/RS256/ES256
- **Multi-IdP auth providers** — pluggable authentication provider system
  supporting Ory Kratos sessions, static API keys, and JWT bearer tokens
- **Kubernetes operational guidance** — `k8s/` manifests for Deployment,
  Service, NetworkPolicy (admin port isolation), and HPA; sticky-session
  warning when `REPLICA_COUNT > 1` without session affinity

[Unreleased]: https://github.com/prometheus/flint-gate/compare/v0.1.0-beta...HEAD
[0.1.0-beta]: https://github.com/prometheus/flint-gate/releases/tag/v0.1.0-beta
