# Analysis — agent-authz-control-plane

**Date:** 2026-07-02
**Phase:** agent-authz-control-plane
**Mode:** stack-specified (Rust workspace; Axum 0.8 / Tokio / Postgres; optional Redis)
**Inputs:** `assessment.md`, seed brief `ai-agent-gateway-parity.md`
**Research:** 3 parallel dependency-research agents (real crates.io/GitHub/docs.rs lookups, 2026-07-02 currency, ~30 queries total)

## Purpose

Resolve the assessment's 5 open build-vs-adopt decisions with evidence before Spec is written. All verdicts below are **High confidence** and source-backed.

---

## Decision 1 — Policy engine (G2): **ADOPT Cedar** (`cedar-policy`)

**Verdict:** Adopt `cedar-policy = "4"` (4.11.2). Fallback: `regorus` (Microsoft, in-process Rego). Reject casbin-rs, oso, OPA-server, OpenFGA.

**Why Cedar over casbin-rs:**
- Cedar core types (`PolicySet`, `Schema`, `Authorizer`, `Entities`) are **`Send + Sync`** → shareable lock-free via `arc_swap::ArcSwap<Arc<CedarBundle>>`. casbin-rs's `Enforcer` is **explicitly not thread-safe** (README) → forces `Arc<RwLock<Enforcer>>`, a write-lock contention hazard on a streaming hot path.
- Cedar's `principal/action/resource/context` quad maps 1:1 onto (identity claims → principal+context, MCP tool name → action, tool params → resource+context). Typed schema; ABAC on `context.*` is its design center. casbin's untyped Rhai-like matcher makes nested-param ABAC second-class and unanalyzable.
- Runtime-editable: `PolicySet::from_str`, `Schema::from_json_str`, `Entities::from_json_*` all parse from **strings at runtime** → store Cedar policy text + schema in Postgres JSONB, parse on hot-reload, atomic `ArcSwap`. First-class supported path (not compile-time bound).
- Deterministic, DoS-resistant by design (no regex/unbounded built-ins) — corroborated by the Teleport×Doyensec SPEF benchmark. Microsecond-class indexed evaluation. Rego (regorus) is more expressive but the same benchmark flags non-determinism + input-DoS risk — wrong default for an untrusted hot path.
- Maintenance: v4.11.2 (2026-06-22), repo pushed 2026-07-02, AWS-backed (powers Verified Permissions), 1.9M recent downloads. Strongest of the set. Apache-2.0.

**Rejected:** oso/Polar — **deprecated** (README says so; last publish 2024-01-13; vendor pivoted to SaaS). OPA-WASM — heavier wasmtime runtime + WASM compile per policy change. OpenFGA — remote/network hop, violates no-sidecar.

**Integration constraints (carry to spec):** wrap compiled bundle in `ArcSwap`; **parse-before-swap, fail-closed** (retain last-good on bad DB policy); validate stored policies against schema at **write-time** in the Admin API (Cedar `Validator`) so bad policy never reaches the hot path; represent each MCP tool call as one Cedar `Request`; for `list_tools`, evaluate per candidate and drop `Deny`.

---

## Decision 2 — Rate limiting + budgets (G3): **HYBRID** (governor + Redis windows)

**Verdict:** Adopt `governor = "0.10"` + `tower_governor = "0.8"` for in-process per-replica request-rate shielding; **hand-roll Redis window counters** (behind the existing `redis-l2` feature) for authoritative, shared per-key/per-team rolling-window token budgets + cross-replica rate limits; keep Postgres `usage_events` as the durable ledger.

**Rationale:**
- `governor` (GCRA, 63.7M downloads, MIT) is **in-process only** — confirmed no Redis/distributed backend exists (deps: crossbeam, hashbrown, portable-atomic). Perfect as a zero-latency local burst shield; **inaccurate across N replicas** (allows ~N× configured rate).
- Shared/accurate enforcement needs a central counter. **Reuse the existing `redis` dep** (already optional behind `redis-l2`, from the L2-cache phase) — a ~15-line Lua fixed/sliding-window script (`INCR`+`EXPIRE` or sorted-set). Token budgets use the same shape: `INCRBY <tokens>` on `budget:{key}:{minute|hour|day}` with TTL, block over threshold. **Do not** add `redis-rate`/`tower-rate-limit-redis` crates — they'd duplicate the redis dep and risk version conflicts.
- **Tradeoff:** governor = 0 network, per-replica loose; Redis = 1 sub-ms RTT, accurate + shared. Hybrid = fast local ceiling + accurate global ceiling.
- **Fallback when `redis-l2` is off:** governor-only rate limiting + a Postgres windowed token check (`SELECT SUM(tokens) … WHERE ts > now()-interval` on `usage_events`, briefly cached). Correct, less hot-path-efficient, zero new infra.

Single-binary honored: governor in-process; Redis is pre-existing optional, not a new sidecar.

---

## Decision 3 — MCP OAuth 2.1 resource-server (G1): **HAND-ROLL surface + reuse jsonwebtoken; optional openidconnect for discovery**

**Verdict:** No single crate covers the MCP RS/metadata role (confirmed against `oauth2`, `openidconnect`, `rmcp`, `jwks`). Best split:

- **Hand-roll (trivial, ~1 handler each):** RFC 9728 `.well-known/oauth-protected-resource` JSON (`axum::Json`); `WWW-Authenticate: Bearer resource_metadata="…"` on 401 + `error="insufficient_scope"` on 403 step-up; RFC 8707 `resource`/audience + scope checks (equality/subset on decoded claims); PKCE S256 check (`base64url(SHA256(verifier)) == challenge` via `sha2`).
- **JWKS discovery + rotation + verify:** reuse the **existing `jsonwebtoken = "9"`** + a small `reqwest`-based cached `HashMap<kid, DecodingKey>` (refresh on unknown `kid`, honor `Cache-Control`) — **leanest, ~60–80 lines, single-binary-friendly**. Optional: adopt `openidconnect = "4"` for RFC 8414/OIDC AS discovery + JWKS if federating many issuers, but its RP-shaped ID-token API needs adaptation for access-token validation.
- **Avoid:** `jwks` crate — pins `jsonwebtoken ^10`, **conflicts with the pinned v9** (would force a breaking bump or duplicate graph). `jwks-cache` — immature + GPL-3.0 (license-incompatible). `oauth2` — client-only (no RS validation). `rmcp` `auth` feature — client-side only (adopt `rmcp` for MCP *transport* if useful, not RS auth).

**Constraint (carry to spec):** stay on `jsonwebtoken = "9"` to avoid the v10 conflict; add only `sha2` (likely already transitively present) + `reqwest` (present) for JWKS.

---

## Decision 4 — Web config UI (G4): **React+Vite SPA (reuse TS SDK) embedded via rust-embed**

**Verdict:**
- **Serving:** `rust-embed = "8"` with its `axum` feature + SPA `index.html` fallback handler; use `debug-embed` (loads from disk in dev, embeds in release) for the dev loop. **Preserves single-binary shipping** — `ServeDir` cannot serve embedded assets (confirmed by axum maintainers, discussion #1309), so it's disqualified by the single-binary value prop. Fallback: `include_dir` if proc-macro compile cost bites; `ServeDir`+disk only if single-binary is relaxed.
- **Frontend:** **React + Vite SPA** importing the **existing TypeScript SDK** directly (built in the prior phase) — zero API re-implementation, stays in lockstep with the utoipa-generated OpenAPI. Charts: **Recharts** for cost/token bar/line/area; **uPlot** only for dense `usage_events` time-series. Code-split analytics so CRUD screens stay under the ~300 KB app-page JS budget.
- **Rejected:** HTMX/Alpine/Lit — throws away the TS SDK and still needs JS for charts. Leptos scaffolds (lapa) — WASM, can't reuse the TS SDK. No Rust-native admin scaffold is mature enough to adopt; port the Vite+rust-embed *pattern* instead.

Mutually reinforcing with the existing utoipa/OpenAPI + multi-SDK ecosystem.

---

## Decision 5 — OAuth2-client/introspection breadth in G1?

**Verdict:** **Defer** to the next phase (as the seed brief already scoped). This phase's G1 is the MCP *resource-server* surface only. Generic OAuth2 client / token-exchange (RFC 8693) / introspection breadth is the NHI/delegation workstream — roadmapped next, once MCP RS + policy engine exist to build on. No change to phase scope.

---

## New dependencies summary (for Cargo.toml)

| Crate | Version | For | New? |
|-------|---------|-----|------|
| `cedar-policy` | `4` | G2 policy engine | new |
| `arc-swap` | `1` | G2 lock-free hot-reload of policy bundle | new (likely) |
| `governor` | `0.10` | G3 in-process rate shield | new |
| `tower_governor` | `0.8` | G3 Axum rate-limit layer | new |
| `rust-embed` | `8` (feat `axum`, `debug-embed`) | G4 embed SPA | new |
| `redis` | `1` (existing, `redis-l2`) | G3 shared window counters | reuse |
| `jsonwebtoken` | `9` (existing) | G1 access-token verify | reuse |
| `reqwest`, `sha2` | existing | G1 JWKS fetch, PKCE | reuse |
| (frontend) React + Vite + Recharts + existing TS SDK | — | G4 UI | new (JS) |

**Explicitly NOT added:** casbin, oso, regorus (fallback only), oauth2, openidconnect (optional only), jwks, jwks-cache, rmcp-auth, redis-rate, include_dir, axum-embed, Leptos.

## Open questions remaining for plan/spec

1. **Redis L2 as a hard dep for accurate budgets?** Recommend keeping it optional with the documented Postgres fallback — plan should decide whether distributed accuracy is a launch requirement or a follow-up.
2. **HITL decision channel (G5):** in-band stream resume vs out-of-band Admin API callback — needs a design spike in the G5 change (leans in-band via AG-UI/A2UI to leverage the moat).
3. **Per-tool Cedar action modeling:** one generic `Action::"call_tool"` with `context.tool_name`, vs a distinct action per tool — recommend generic action + context (avoids schema churn as tools change); confirm in spec.
