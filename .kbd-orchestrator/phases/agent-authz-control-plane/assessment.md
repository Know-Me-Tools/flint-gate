# Assessment — agent-authz-control-plane

**Date:** 2026-07-02
**Phase:** agent-authz-control-plane
**Project:** flint-gate — AI-native auth proxy / API gateway (Rust workspace, ~7.8k LOC)
**Seed:** `.kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md`
**Prior phase:** sdk-ecosystem-and-docs (completed — 12/12 changes, tests green)

## Method

Inspected the workspace against the 6 phase goals. Verified each claimed gap by
grepping the codebase, not by assumption. Cross-tool progress: none since last
session (this is a fresh phase; `progress.json.changes = []`).

## Current-state snapshot (verified from code)

- **Workspace:** 3 crates — `flint-gate` (bin), `flint-gate-core` (lib), `flint-gate-client` (SDK). Axum 0.8, Tokio, `tower-http` 0.6, `utoipa` + `utoipa-swagger-ui` 9 already present.
- **Auth providers** (`config/types.rs::AuthProviderConfig`): `Kratos`, `Jwt` (JWKS verify in `auth/jwt_verify.rs`), `ApiKey` (Postgres, SHA-256 hashed), `Anonymous`. Route-selectable (route override → site default → anonymous).
- **Hooks** (`config/types.rs`): `PreRequestHook::{ClaimsEnhancement, BodyTransform, MaxTokenBudget}`; `PostResponseHook::StreamMeter`.
- **Streaming** (`stream/`): `processor.rs` (SSE/AG-UI), `a2ui.rs` (A2UI intents), `ndjson.rs`, `websocket.rs`. Mid-stream token metering + session watchdog. **This is the moat.**
- **DB** (`db/mod.rs`): schema applied at startup via idempotent `migrate()` (`CREATE TABLE IF NOT EXISTS`). Tables: `gate_routes` (JSONB), `gate_sites`, `api_keys`, `usage_events`, `jwt_signing_keys`. **No external migration framework — new tables follow the same `migrate()` pattern.**
- **Admin server** (`main.rs`): routes `/health`, `/ready`, `/cache/stats`, `/cache/invalidate`, route CRUD. Serves Swagger UI (utoipa) — precedent for serving an SPA.
- **Config:** YAML + env + CLI precedence; hot-reload (filesystem watcher + Postgres LISTEN/NOTIFY).

## Gap report (per goal)

### G1 — MCP OAuth 2.1 resource-server  ·  **NOT STARTED** (CRITICAL)
- **Present:** `auth/jwt_verify.rs` does JWKS-based JWT verification (issuer/audience/JWKS URL) — reusable for token validation.
- **Missing (all):** RFC 9728 protected-resource metadata endpoint (`.well-known/oauth-protected-resource`), `WWW-Authenticate: resource_metadata=…` on 401, RFC 8414/OIDC AS discovery, PKCE S256 verification, RFC 8707 `resource`/audience binding, 403 `insufficient_scope` step-up, confused-deputy / no-token-passthrough guard.
- **Effort:** medium. Mostly greenfield endpoints + a new auth provider variant (`Mcp`), reusing JWKS verify. No new heavy deps (an OAuth2/JOSE crate may help).
- **Files touched:** new `auth/mcp.rs`; `config/types.rs` (new provider); admin/proxy router for `.well-known`; `auth/jwt_verify.rs` (reuse).

### G2 — Embedded policy engine + per-tool-call authz  ·  **NOT STARTED** (CRITICAL)
- **Present:** only client-SDK `policy`-named types (`flint-gate-client`) — NOT an engine. Authorization today is route-level allow/deny by auth-provider match.
- **Missing (all):** a policy engine; per-tool-call authorization by tool name + params + identity claims; `list_tools` filtering of unauthorized tools; a `PreRequestHook::Authorize` variant; a stream-level tool-call gate.
- **Decision needed (open question for analyze/plan):** **Cedar** (typed, formally analyzable, AWS, native Rust core) vs **casbin-rs** (mature multi-model ABAC/ReBAC). Both embed in-process (no sidecar — preserves single-binary value prop). Recommend Cedar for typed/analyzable policies; casbin-rs if ReBAC-by-relationship-tuples is the priority.
- **Effort:** large — the strategic core. Depends on G1 for identity claims plumbing.
- **Files touched:** new `authz/` module (engine wrapper); `config/types.rs` (policy attach + `Authorize` hook); `stream/processor.rs` (tool-call gate + `list_tools` filter); `middleware/pipeline.rs`.

### G3 — Budget enforcement + windowed rate limiting  ·  **PARTIAL** (HIGH) — best starting point
- **Present:** `usage_events` table (per-request token/duration metering); `PreRequestHook::MaxTokenBudget` blocks on **lifetime** usage; `StreamMeter` post-response hook writes usage.
- **Missing:** windowed (minute/hour/day) budgets; per-key / per-team scoping; **request-rate limiting** (no `governor`/token-bucket dependency present); enforcement primitives beyond a single lifetime cap.
- **Effort:** small-medium — extends existing code. Highest feasibility, fastest win. Likely add `governor` (or a Redis-window counter) for rate limits; extend `MaxTokenBudgetConfig` with a `window` + `scope`.
- **Files touched:** `config/types.rs` (extend hook config); `config/lookup.rs`; `db/mod.rs` (windowed usage query, maybe a rollup); `middleware/pipeline.rs`.

### G4 — Web configuration + observability UI  ·  **NOT STARTED** (HIGH)
- **Present:** Admin API does route/auth/cache CRUD; utoipa Swagger UI is served (SPA-serving precedent). `tower-http` present.
- **Missing:** the SPA itself; `ServeDir` static serving (needs `tower-http` `fs` feature); observability endpoints aggregating `usage_events` (token/cost analytics); UI management for hooks/policies/budgets/audit.
- **Effort:** medium. New frontend (framework TBD — align with existing SDK/docs stack) + a handful of read-model Admin endpoints. Best built **last** so it manages real features from G1–G3, G6.
- **Open question for plan:** frontend framework + whether to embed built assets in the binary (`include_dir`/`rust-embed`) or serve from disk.

### G5 — Human-in-the-loop approval gates  ·  **NOT STARTED** (HIGH — differentiator)
- **Present:** AG-UI event processing + A2UI intent filtering in the stream — the ideal surface to render approval prompts. WebSocket + SSE plumbing exists.
- **Missing (all):** pause/resume of a flagged tool call; an approval-request event over AG-UI/A2UI; a resume/abort decision channel; approval state store.
- **Effort:** medium-large. Depends on G2 (a policy decision can *require* approval). Novel — few competitors ship this well; leverages the moat.
- **Files touched:** new `authz/approval.rs`; `stream/processor.rs` + `stream/a2ui.rs` (emit approval event, await decision); Admin API (approval decision endpoint) or in-band resume.

### G6 — Guardrail hook interface + authz decision audit trail  ·  **NOT STARTED** (MEDIUM — cross-cutting)
- **Present:** the `PreRequestHook` enum is the natural extension point; A2UI intent filtering is a partial guardrail precedent.
- **Missing:** a pluggable guardrail hook interface (defer bundling injection/PII models); an `authz_audit` table + write path capturing every allow/deny/step-up/approval; Admin API + UI surfacing.
- **Effort:** small-medium. `authz_audit` table follows the `migrate()` pattern. Fold in as the engine (G2) lands.
- **Files touched:** `config/types.rs` (guardrail hook variant); new audit write in `authz/`; `db/mod.rs` (`authz_audit` table); Admin API read endpoint.

## Gap severity summary

| Goal | Status | Severity | Effort | Feasibility | Best sequence |
|------|--------|----------|--------|-------------|---------------|
| G3 Budget + rate limit | PARTIAL | HIGH | S–M | High | **1st** (fastest win) |
| G1 MCP OAuth2 RS | NOT STARTED | CRITICAL | M | Med-High | 2nd (credibility gate) |
| G2 Policy engine + per-tool authz | NOT STARTED | CRITICAL | L | Med | 3rd (strategic core; needs G1 claims) |
| G5 HITL approval | NOT STARTED | HIGH | M–L | Med | 4th (needs G2) |
| G4 Web config UI | NOT STARTED | HIGH | M | Med | 5th (manages G1–G3, G6) |
| G6 Guardrail hook + audit | NOT STARTED | MEDIUM | S–M | High | folded across 2–3 |

## Architectural notes / constraints carried forward

- **Single-binary, no-sidecar is a core value prop** → prefer *embedded* policy engine (Cedar/casbin-rs) over remote PDP (OpenFGA/Cerbos). Revisit remote PDP only if ReBAC relationship data outgrows in-process.
- **Schema changes go through `migrate()`** (idempotent `CREATE TABLE IF NOT EXISTS`) — no Diesel/sqlx-migrate framework. New tables: `authz_policies`, `authz_audit`, possibly `usage_rollups`, `approvals`.
- **Do NOT enter the LLM-ops bundle** (semantic caching / multi-LLM routing / multimodal) — off-identity per the seed brief. Guardrails ship as a *hook interface*, not bundled models.
- **Reuse the stream moat:** G5 (HITL) and G2 (`list_tools` filtering, tool-call gate) should be implemented inside the existing `stream/processor.rs` + `a2ui.rs`, not bolted alongside.

## Open questions for analyze / plan

1. **Policy engine:** Cedar vs casbin-rs? (Recommend Cedar — typed, analyzable, native Rust; decide in analyze.)
2. **Rate-limit backend:** in-process `governor` vs Redis-backed window counters (Redis L2 cache already exists from a prior phase — reuse for distributed limits?).
3. **Web UI framework** and **asset embedding** (in-binary vs disk).
4. **HITL decision channel:** in-band stream resume vs out-of-band Admin API callback.
5. Should G1 (MCP RS) also add generic **OAuth2 client / introspection** breadth now, or defer to the next phase (currently out of scope)?

## Verdict

The phase goals are well-founded and mostly greenfield, but built on strong
existing foundations: `usage_events`/`MaxTokenBudget` (G3), JWKS verify (G1),
the hook enum (G2/G6), and the AG-UI/A2UI stream (G2/G5). No goal is blocked;
recommended entry point is **G3** (lowest effort, extends existing code),
then G1 → G2 → G5 → G4, folding G6 across. Proceed to analyze (engine + backend
decisions) then plan.
