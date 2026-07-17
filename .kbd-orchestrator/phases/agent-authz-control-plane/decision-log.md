# Decision Log — agent-authz-control-plane

### 2026-07-02 — Analyze: build-vs-adopt calls (all High confidence, source-backed)

- **D-A01 · Policy engine → ADOPT Cedar** (`cedar-policy@4`). [analyze · 2026-07-02]
  TL;DR: only native-Rust, Send+Sync, runtime-loadable, deterministic option.
  Why: casbin-rs enforcer not thread-safe (RwLock hot-path hazard); oso deprecated; OPA/OpenFGA break no-sidecar.
  Alternatives: regorus (fallback), casbin (rejected), oso (deprecated). Provenance: research.
- **D-A02 · Rate limit/budgets → HYBRID** governor + hand-rolled Redis windows. [analyze · 2026-07-02]
  TL;DR: in-process burst shield + authoritative shared Redis counters (reuse existing redis dep); Postgres usage_events = ledger.
  Why: governor has no distributed backend; adding a redis-rate crate would duplicate/conflict deps. Provenance: research.
- **D-A03 · MCP resource-server → HAND-ROLL surface + reuse jsonwebtoken@9.** [analyze · 2026-07-02]
  TL;DR: no crate covers RS/metadata role; .well-known + WWW-Authenticate + RFC 8707 + PKCE are trivial; JWKS on existing jwt lib.
  Why: `jwks` crate pins jsonwebtoken ^10 (conflicts pinned v9); oauth2/rmcp-auth are client-only. Provenance: research.
- **D-A04 · Web config UI → React+Vite SPA (reuse TS SDK) embedded via rust-embed@8.** [analyze · 2026-07-02]
  TL;DR: rust-embed preserves single-binary (ServeDir can't serve embedded); SPA reuses existing TS SDK; Recharts/uPlot for analytics.
  Why: HTMX/Leptos forfeit the TS SDK. Provenance: research.
- **D-A05 · OAuth2-client/introspection breadth → DEFER to next phase.** [analyze · 2026-07-02]
  TL;DR: NHI/delegation (RFC 8693) is next-phase scope per seed brief; this phase = MCP RS surface only. Provenance: seed brief.
