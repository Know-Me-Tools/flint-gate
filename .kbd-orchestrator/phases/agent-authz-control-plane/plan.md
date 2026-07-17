# Plan — agent-authz-control-plane

**Date:** 2026-07-03
**Phase:** agent-authz-control-plane
**Backend:** OpenSpec (`openspec/` present; `openspec_available: true`)
**Inputs:** `assessment.md`, `analysis.md`, `library-candidates.json`, seed `ai-agent-gateway-parity.md`
**Evolver bridge:** none (not an evolver-driven cycle)

## Ordering rationale

Build order is impact-weighted **and** dependency-aware, per the analysis:
lowest-effort/highest-feasibility first to de-risk, then the strategic core,
then the surfaces that manage them. Each change reuses the analyzed
library-candidate decision (no rebuild of adopted libs). G6 (guardrail hook +
audit trail) is split: the **audit trail** lands with the policy engine (it has
no value until there are authz decisions to record), and the **guardrail hook
interface** rides as a small standalone change late in the sequence.

Dependencies: `007` (per-tool authz) needs `006` (MCP identity claims plumbed);
`008` (HITL) needs `007` (a policy decision that *requires* approval);
`009` (web UI) is last so it manages real features from `004`–`008`, `010`.

## Ordered change list

| # | Change ID | Goal | Library decision | Depends on | Effort |
|---|-----------|------|------------------|-----------|--------|
| 1 | `add-budget-rate-limiting` | G3 | governor@0.10 + tower_governor@0.8 (adopt) + hand-rolled Redis windows (reuse `redis`) | — | S–M |
| 2 | `add-mcp-resource-server` | G1 | hand-roll RS surface + reuse `jsonwebtoken@9`; `sha2`/`reqwest` (reuse) | — | M |
| 3 | `add-policy-engine` | G2a | **cedar-policy@4** (adopt) + `arc-swap@1` | 2 (claims) | M–L |
| 4 | `add-per-tool-authz` | G2b | cedar (from 3); stream tool-call gate + `list_tools` filter | 3 | M–L |
| 5 | `add-authz-audit-trail` | G6b/G8 | none (new `authz_audit` table via `migrate()`) | 3 | S |
| 6 | `add-hitl-approval` | G5 | none (in-band AG-UI/A2UI resume) | 4 | M–L |
| 7 | `add-guardrail-hook` | G6a | none (pluggable `PreRequestHook` variant; no bundled model) | — | S–M |
| 8 | `add-web-config-ui` | G4 | **rust-embed@8** + React/Vite SPA (reuse TS SDK) + Recharts | 1–7 surfaced | M |

8 changes total.

---

## Change details

### 1 — `add-budget-rate-limiting`  (G3)  · library: governor + redis-windows
**Why first:** extends existing `usage_events` + `MaxTokenBudget` hook; highest feasibility, fastest win.
- Extend `MaxTokenBudgetConfig` with `window` (minute/hour/day) + `scope` (key/team) beyond the lifetime cap.
- Add `governor` + `tower_governor` in-process request-rate layer on the proxy router.
- Hand-roll Redis Lua window counters behind the existing `redis-l2` feature for authoritative shared token budgets + cross-replica rate limits; block over threshold with a typed error.
- Fallback path when `redis-l2` off: governor-only + Postgres windowed `SUM(tokens)` check (briefly cached).
- Files: `config/types.rs`, `config/lookup.rs`, `db/mod.rs`, `middleware/pipeline.rs`, new `ratelimit/`.
- Agent: `rust` (general) with `rust-reviewer` after.

### 2 — `add-mcp-resource-server`  (G1)  · library: hand-roll + jsonwebtoken@9
**Why second:** credibility gate; plumbs the identity claims that `003`/`004` consume.
- New auth provider variant `Mcp` (or extend Jwt) validating OAuth 2.1 access tokens.
- `.well-known/oauth-protected-resource` (RFC 9728) JSON handler; `WWW-Authenticate: Bearer resource_metadata="…"` on 401; `error="insufficient_scope"` 403 step-up.
- RFC 8707 `resource`/audience validation + scope checks; PKCE S256 check via `sha2`.
- JWKS fetch/rotation: cached `HashMap<kid, DecodingKey>` on existing `jsonwebtoken@9` (**do not** bump to v10; **do not** add `jwks` crate).
- No token passthrough to upstream (confused-deputy guard).
- Files: new `auth/mcp.rs`, `auth/jwks.rs`; `config/types.rs`; proxy/admin router; reuse `auth/jwt_verify.rs`.
- Agent: `rust` + `security-reviewer` (auth code).

### 3 — `add-policy-engine`  (G2a)  · library: cedar-policy@4 + arc-swap
- New `authz/` module wrapping Cedar: `PolicySet` + `Schema` + `Entities`, shared via `arc_swap::ArcSwap<Arc<CedarBundle>>`.
- Store policy text + schema in Postgres JSONB; hot-reload = parse-before-swap, **fail-closed** (retain last-good).
- Write-time validation via Cedar `Validator` in the Admin API so bad policy never reaches the hot path.
- New `authz_policies` table via idempotent `migrate()`.
- New `PreRequestHook::Authorize` variant (route-level decisions).
- Files: new `authz/{mod,engine,reload}.rs`; `config/types.rs`; `db/mod.rs`; admin router (policy CRUD + validate).
- Agent: `rust` + `rust-reviewer`.

### 4 — `add-per-tool-authz`  (G2b)  · library: cedar (from 3)
**Depends on 3.** Strategic core.
- Represent each MCP tool call as one Cedar `Request` (principal=identity, action=generic `call_tool`, resource=tool, `context`=params + `tool_name`).
- Inline evaluation in `stream/processor.rs`; deny → block the tool call.
- Filter unauthorized tools out of MCP `list_tools` responses (evaluate per candidate, drop `Deny`).
- Files: `stream/processor.rs`, `stream/a2ui.rs`, `authz/`, `middleware/pipeline.rs`.
- Agent: `rust` + `rust-reviewer`.

### 5 — `add-authz-audit-trail`  (G6b/G8)  · library: none
**Depends on 3.** Small; lands right after the engine so every decision is recorded.
- New `authz_audit` table via `migrate()`; write every allow/deny/step-up/approval with principal, action, resource, decision, reason, ts.
- Admin API read endpoint (paged, filterable).
- Files: `db/mod.rs`, `authz/audit.rs`, admin router.
- Agent: `rust`.

### 6 — `add-hitl-approval`  (G5)  · library: none  · differentiator
**Depends on 4.** Design spike inside the change: in-band vs Admin callback (lean **in-band**).
- A Cedar decision may return `RequireApproval`; pause the flagged tool call.
- Emit an approval-request event over the existing AG-UI/A2UI stream; await approve/deny on a decision channel; resume or abort.
- Approval state store (short-lived; Postgres or in-memory + Redis).
- Files: new `authz/approval.rs`; `stream/processor.rs`, `stream/a2ui.rs`; admin/decision endpoint.
- Agent: `rust` + `rust-reviewer`; prove end-to-end in an example.

### 7 — `add-guardrail-hook`  (G6a)  · library: none
Standalone; no dependency.
- New pluggable guardrail `PreRequestHook` variant with a trait interface. **Defer bundling injection/PII models** — ship the interface + a trivial reference guard (e.g., regex/allowlist) only.
- Files: `config/types.rs`, `middleware/pipeline.rs`, new `guardrail/`.
- Agent: `rust`.

### 8 — `add-web-config-ui`  (G4)  · library: rust-embed@8 + React/Vite + Recharts
**Last** — manages routes/auth/hooks/policies/budgets/audit built above.
- React+Vite SPA importing the existing TypeScript SDK; embed the `dist/` via `rust-embed@8` (`axum` + `debug-embed` features) served by the Admin server with SPA `index.html` fallback.
- Read-model Admin endpoints aggregating `usage_events` (token/cost analytics) + `authz_audit`.
- Charts: Recharts (cost/token); uPlot only for dense time-series. Code-split analytics (<300 KB app-page budget).
- Files: new `web/` frontend dir; admin router (static serve + read models); `Cargo.toml` (`rust-embed`).
- Agent: `rust` (backend) + frontend; `typescript-reviewer` for the SPA.

---

## Cross-cutting constraints (from analysis)

- **Single-binary, no sidecar** — embedded Cedar, in-process governor, embedded SPA. Redis stays optional (`redis-l2`).
- **Schema via idempotent `migrate()`** — new tables: `authz_policies`, `authz_audit`, `approvals`. No migration framework.
- **Stay on `jsonwebtoken@9`** — avoid the `jwks`-crate v10 conflict.
- **Do NOT enter the LLM-ops bundle** — guardrails ship as an interface, not bundled models.
- **Reuse the stream moat** — per-tool authz + HITL live inside `stream/processor.rs`/`a2ui.rs`.

## First change to apply

`add-budget-rate-limiting` (change 1) — lowest risk, extends existing code, no dependencies.

## Open questions carried to spec

1. Redis L2 as hard dep for accurate budgets, or optional with Postgres fallback? (recommend optional)
2. HITL decision channel: in-band stream resume vs Admin callback (recommend in-band) — resolve in change 6 spec.
3. Cedar action modeling: generic `call_tool` + `context.tool_name` vs per-tool action (recommend generic) — resolve in change 3/4 spec.
