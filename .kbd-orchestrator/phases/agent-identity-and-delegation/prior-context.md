# Prior context — agent-identity-and-delegation

Seeded from **agent-authz-control-plane** (6/6 goals MET, 8/8 changes archived).

## What already exists to build on
- MCP OAuth 2.1 **resource-server** surface (RFC 9728 PRM, RFC 8707 audience,
  PKCE S256, no token passthrough) — `add-mcp-resource-server`.
- Embedded **Cedar policy engine** with ArcSwap hot-reload + write-time
  validation, and **per-tool-call authorization** (name+params+claims,
  `list_tools` filtering) — `add-policy-engine`, `add-per-tool-authz`.
- Windowed **token budgets + rate limiting** (governor + Redis + Postgres
  ledger) — `add-budget-rate-limiting`.
- **HITL approval** gates over AG-UI/A2UI — `add-hitl-approval`.
- **Authz audit trail** + **guardrail hook** interface — `add-authz-audit-trail`,
  `add-guardrail-hook`.
- **Web config + observability UI** (rust-embed SPA) — `add-web-config-ui`.

## Debt this phase must address
- **Admin API is unauthenticated (loopback-only)** — top priority, gates
  exposing the new web UI beyond `127.0.0.1`. → Goal 1.
- Redis-L2-as-hard-dep for accurate cross-replica budgets — unresolved.

See `../agent-authz-control-plane/reflection.md` for the full record.
