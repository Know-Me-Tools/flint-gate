# Evolution Brief: AI-Agent Gateway Parity & Differentiation

**Generated:** 2026-07-02
**Project:** flint-gate — AI-native auth proxy / API gateway (Rust), positioned as an Ory Oathkeeper replacement for streaming LLM workloads. By KnowMe, LLC.
**Criteria profile:** effort-impact (impact 40% · effort 25% inverted · alignment 20% · feasibility 15%)
**Research depth:** deep (3 parallel research clusters, ~40+ web sources, 2025–2026 window)

---

## TL;DR

Flint-gate has a genuine, defensible differentiator that **no competitor matches**: mid-stream enforcement over SSE (live token metering + a session watchdog that kills active streams on session expiry), plus AG-UI/A2UI protocol processing. The MCP spec itself flags mid-stream scope/expiry enforcement as an *unsolved* problem — flint-gate already solves the hard part.

But the market converged in 2025–2026 on a standard "AI gateway" bundle that flint-gate is missing almost entirely. The credibility-defining gaps, in priority order:

1. **MCP authorization (OAuth 2.1 resource-server model)** — the single non-negotiable 2026 standard. flint-gate has zero of it.
2. **Per-tool-call authorization + a policy engine** — flint-gate's authz is coarse (route-level only).
3. **Token/cost budgets with enforcement + rate limiting** — flint-gate *meters* but barely *enforces* (only a lifetime `MaxTokenBudget` hook; no windowed rate limiting).
4. **A web configuration + observability UI** — 12 of 14 competitors ship one; flint-gate has none.
5. **Guardrails (prompt-injection / PII-DLP)** and **multi-LLM routing + semantic caching** — table stakes for "AI gateway," but a strategic *fork in the road* for flint-gate (see "Scope — what's out").

---

## Selected evolution: **Become a credible MCP-era agent gateway — authorization first, config UI second**

Ship, in one phase, the "agent authorization control plane" that turns flint-gate from an *auth proxy* into an *agent gateway*, built on top of the streaming enforcement it already owns:

- **MCP OAuth 2.1 resource-server support** (RFC 9728 protected-resource metadata, `WWW-Authenticate` discovery, RFC 8707 resource/audience validation, PKCE verification, 403 `insufficient_scope` step-up).
- **Per-tool-call authorization** using an *embedded native-Rust policy engine* (Cedar or casbin-rs — no sidecar), evaluated inline in the stream, filtering unauthorized tools from MCP `list_tools` (the agentgateway pattern).
- **Token/cost budgets with real enforcement + windowed rate limiting** (extend the existing `usage_events` + `MaxTokenBudget` foundation into per-key/per-team/per-window budgets and request-rate limits).
- **A web configuration + observability dashboard** — routes, auth providers, hooks, policies, and live token/cost analytics, served from the Admin API (which already does Postgres route CRUD).
- **Human-in-the-loop approval gates** rendered through the *existing* AG-UI/A2UI stream — the differentiator few competitors ship well, and a natural fit for flint-gate's architecture.

### Why this, why now

The market has a clear, narrow window: MCP authorization only crystallized into a hard spec in mid-2025 (2025-06-18 → 2025-11-25 revisions), and per-tool agent authorization is still being figured out by everyone. Flint-gate is *already* the only proxy doing mid-stream SSE enforcement — the exact capability the MCP spec calls unsolved. Adding MCP resource-server support + per-tool authz on that foundation produces something genuinely differentiated ("the gateway that enforces agent authorization *during* the stream, not just at connect"), rather than a me-too AI gateway chasing Kong/Cloudflare on semantic caching. Authorization is also aligned with flint-gate's identity (Oathkeeper replacement) — it is authz, not a pivot into LLM-ops.

### Scope

**In:**
- MCP OAuth 2.1 resource-server primitives (RFC 9728, 8707, 8414 discovery, PKCE, step-up 403).
- Embedded policy engine (Cedar Rust core *or* casbin-rs) → per-route + per-tool-call decisions; new `PreRequestHook::Authorize` + stream-level tool-call gate.
- Tool filtering from MCP `list_tools` for unauthorized principals.
- Budget/rate-limit enforcement: extend `MaxTokenBudget` (currently lifetime-only) to windowed (per minute/hour/day) + per-key/per-team + request-rate limiting; block on threshold.
- HITL approval gate: pause a flagged tool call, emit an approval request over AG-UI/A2UI, resume on approve/deny.
- Web config + observability UI (served by Admin server; SPA against Admin API).
- Authorization decision audit trail (new table + Admin API surface).
- Non-human identity primitives: short-lived, audience-bound per-agent tokens; task-scoped grants (zero-standing-privilege direction).

**Out (deliberately deferred — these are the "AI-ops" bundle, a different strategic bet):**
- Semantic caching, multi-LLM routing/failover/load-balancing, prompt compression, multimodal, prompt management/versioning. These are Kong/Portkey/Cloudflare's game; entering it turns flint-gate into an LLM-ops product and dilutes the auth-proxy identity. Revisit as a *separate* phase only if product direction shifts toward LLM-ops.
- Full SAML/SCIM/LDAP federation and becoming an OAuth2 *authorization server* (that is Ory Hydra/Keycloak/Auth0 territory; flint-gate should *federate with* them, not replace them).
- Prompt-injection ML guardrails / PII-DLP as native models — start with a pluggable guardrail *hook* interface; defer bundling a model.

### Success criteria

- [ ] flint-gate passes an MCP client's authorization handshake: returns RFC 9728 protected-resource metadata, emits `WWW-Authenticate` with `resource_metadata` on 401, validates RFC 8707 audience, verifies PKCE, and honors a 403 `insufficient_scope` step-up.
- [ ] A route can attach a policy (Cedar/casbin) that authorizes an MCP tool call by tool name + parameters + identity claims; unauthorized tools are removed from `list_tools` responses.
- [ ] A per-key/per-team token budget over a rolling window blocks the request with a clear error when exceeded (verified by test), and a request-rate limit does the same.
- [ ] A flagged tool call pauses, surfaces an approval prompt via an AG-UI/A2UI event, and resumes or aborts based on the response — proven end-to-end in an example.
- [ ] The web UI can view/create/edit routes, auth providers, hooks, and policies, and shows live token/cost usage from `usage_events`.
- [ ] Every authorization decision (allow/deny/step-up/approval) is written to an audit table and visible via Admin API + UI.
- [ ] Workspace still builds clean: `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` all green; new features covered ≥80%.

### Landscape context (key findings that drove this choice)

- **MCP authorization is the defining 2026 standard.** MCP servers are OAuth 2.1 *resource servers* (separate AS). Hard requirements: RFC 9728 protected-resource metadata, RFC 8414/OIDC AS discovery, PKCE S256, RFC 8707 `resource`/audience binding, and **no token passthrough** (confused-deputy prevention). WorkOS, Stytch, Descope, Auth0 already ship these; flint-gate has none. (modelcontextprotocol.io 2025-11-25; descope.com; workos.com; stackoverflow.blog 2026-01.)
- **Per-tool-call authz is table stakes and has a concrete blueprint.** *agentgateway* (Agentic AI Foundation) evaluates CEL rules per MCP method against `mcp.tool.name`/target + JWT claims and **filters unauthorized tools out of `list_tools`**. OpenFGA models agents as first-class principals with task-based (expiring, turn-limited, agent-bound) grants; Cerbos and Permit.io ship MCP authz. (agentgateway.dev; openfga.dev/docs/modeling/agents; cerbos.dev; permit.io.)
- **Native-Rust enforcement avoids a sidecar.** `casbin-rs` (mature multi-model ABAC/ReBAC library) and **Cedar** (AWS, typed + formally analyzable, Rust core) both embed in-process — ideal for a Rust proxy. OpenFGA/Cerbos remain options as remote PDPs if relationship data grows. (casbin.org; cedarpolicy.com.)
- **The AI-gateway bundle competitors converged on** — semantic caching, multi-LLM routing/failover, token/cost budgets w/ enforcement, guardrails (prompt-injection + PII/DLP), MCP gateway — is present in Kong, Tyk, APISIX, Cloudflare, Portkey, LiteLLM, Higress, Gloo/agentgateway, Envoy AI, Bifrost. flint-gate is missing all five. **We deliberately take only budgets+MCP+guardrail-hook** and leave caching/routing/multimodal out to protect the auth-proxy identity. (zuplo 2026 comparison; konghq.com; cloudflare docs; higress.ai.)
- **A web config/observability UI is near-universal** (12 of 14 competitors ship one, including lightweight ones — Cloudflare, Portkey, LiteLLM, Bifrost). flint-gate is config-file/Admin-API only. This is a clear, expected gap — hence "yes, add a web config tool" as the user asked. (per gateway dashboards: Kong Manager, Tyk Dashboard, APISIX Dashboard, etc.)
- **HITL approval is emerging as invocation-level authz** ("deterministic pre-action authorization") and few gateways ship it well — a differentiation opportunity that fits flint-gate's stream model. (scalekit.com; arxiv 2603.20953.)
- **Identity systems to integrate with (validate tokens from / federate):** Ory (Kratos ✓ already, Hydra, Keto, Polis), Auth0/Okta (+ "Auth0 for AI Agents": Token Vault, CIBA, Auth for MCP, FGA), Keycloak, Zitadel, FusionAuth, WorkOS, Clerk, Stytch, SuperTokens, Azure AD/Entra, Google, Cognito. Any RFC 8414/9728-compliant AS can be an MCP authorization server flint-gate federates with as a resource server.

---

## Gap matrix (flint-gate vs. landscape)

| # | Capability gap | flint-gate today | Landscape benchmark | Severity |
|---|----------------|------------------|---------------------|----------|
| G1 | MCP authorization (OAuth 2.1 RS) | none | WorkOS, Stytch, Descope, Auth0, agentgateway | **CRITICAL** |
| G2 | Per-tool-call authorization + policy engine | route-level allow/deny only | agentgateway, OpenFGA, Cerbos, Permit, OPA | **CRITICAL** |
| G3 | Token/cost budgets w/ enforcement + rate limiting | lifetime `MaxTokenBudget` only; no windowed/rate limits | Kong, Tyk, Cloudflare, LiteLLM, Envoy AI | **HIGH** |
| G4 | Web config + observability UI | none (Admin API + YAML/CLI) | 12 of 14 competitors | **HIGH** |
| G5 | HITL approval gates | none | Scalekit, LangGraph, gateway "approval desks" | **HIGH (differentiator)** |
| G6 | Guardrails: prompt-injection / PII-DLP | A2UI intent filter (partial) | Kong, APISIX, Cloudflare, Portkey, Gloo | MEDIUM (hook interface only) |
| G7 | NHI / agent identity + delegation (RFC 8693) | coarse JWT minting | Auth0 Token Vault/CIBA, SPIFFE, OpenFGA task grants | MEDIUM |
| G8 | Authorization decision audit trail | none | Cerbos, Permit, OpenFGA, Kong | MEDIUM |
| G9 | OAuth2/OIDC client + introspection (token federation breadth) | JWT/JWKS verify only | Oathkeeper, all IdPs | MEDIUM |
| G10 | Multi-LLM routing / failover / semantic caching / multimodal | none | Kong, Portkey, Cloudflare, Higress, Bifrost | LOW (out of scope — different bet) |

---

## Ranked evolution candidates (effort-impact scored, 1–5 scale)

Score = (impact×0.40) + ((6−effort)×0.25) + (alignment×0.20) + (feasibility×0.15)

| # | Candidate | Impact | Effort | Align | Feas | **Score** |
|---|-----------|:------:|:------:|:-----:|:----:|:---------:|
| 1 | **MCP OAuth 2.1 resource-server support** (G1) | 5 | 3 | 5 | 4 | **4.35** |
| 2 | **Per-tool-call authz + embedded Cedar/casbin engine** (G2) | 5 | 4 | 5 | 4 | **4.10** |
| 3 | **Budget enforcement + windowed rate limiting** (G3) | 4 | 2 | 5 | 5 | **4.35** |
| 4 | **Web config + observability UI** (G4) | 4 | 3 | 4 | 4 | **3.75** |
| 5 | **HITL approval over AG-UI/A2UI** (G5, differentiator) | 4 | 3 | 5 | 4 | **3.95** |
| 6 | Guardrail hook interface (G6, pluggable, no bundled model) | 3 | 2 | 4 | 5 | **3.55** |
| 7 | NHI / task-scoped grants + RFC 8693 delegation (G7) | 3 | 4 | 4 | 3 | **2.85** |
| — | Semantic caching / multi-LLM routing (G10) | 4 | 5 | 2 | 3 | **2.50** (deferred: off-identity) |

**Interpretation:** #1 (MCP RS) and #3 (budget enforcement) tie at 4.35 — MCP is highest *impact/alignment*; budget enforcement is highest *feasibility/lowest effort* (it extends code that already exists). #2 (per-tool authz) is the strategic core but the largest single lift. Recommended build order sequences low-effort/high-feasibility work first to de-risk, then the strategic core.

### Recommended build order (impact-weighted, dependency-aware)

1. **Budget enforcement + rate limiting** (G3) — fastest win, extends existing `usage_events`/`MaxTokenBudget`; immediate value; low risk.
2. **MCP OAuth 2.1 resource-server** (G1) — the credibility gate; unlocks MCP positioning.
3. **Embedded policy engine + per-tool-call authz + `list_tools` filtering** (G2) — the strategic differentiator core; depends on identity claims plumbed by G1.
4. **HITL approval over AG-UI/A2UI** (G5) — builds on G2 (a policy decision can require approval) and flint-gate's existing stream protocols.
5. **Web config + observability UI** (G4) — surfaces routes/policies/budgets/audit built in 1–4; best last so it has real features to manage.
6. **Guardrail hook interface** (G6) + **audit trail** (G8) — cross-cutting, folded in as the engine lands.
7. (Roadmap, next phase) NHI/delegation (G7), OAuth2 client/introspection breadth (G9).

### Impact statements (per top feature)

- **MCP resource-server (G1):** Without it, flint-gate cannot honestly claim to be an "AI-native" or MCP gateway in 2026 — it's the one standard every serious competitor implements. With it, flint-gate becomes usable as the enforcement point in front of any MCP server, federating with any compliant IdP (Ory, Auth0, WorkOS…).
- **Per-tool-call authz (G2):** Converts flint-gate from "authenticates the connection" to "authorizes each agent action" — the difference between an auth proxy and an *agent gateway*. Embedded Cedar/casbin keeps it a single lightweight Rust binary (no sidecar), preserving the core value prop.
- **Budget enforcement + rate limiting (G3):** Turns passive metering into active cost control — the #1 operational ask for anyone running agents at scale, and cheap to add on the existing foundation.
- **HITL approval (G5):** The standout differentiator — deterministic pre-action approval delivered *in-stream* via AG-UI/A2UI, which almost no competitor does well and which flint-gate is uniquely architected for.
- **Web config UI (G4):** Closes the most visible "looks unfinished next to competitors" gap and becomes the home for the token/cost analytics and policy management the above features generate. **Answer to the user's explicit question: yes, add a web-based configuration tool — it is expected table stakes and the natural surface for the new authz/budget features.**

## Runners-up (not selected as the phase's spine)

| # | Title | Score | Why not selected |
|---|-------|-------|------------------|
| 6 | Guardrail hook interface | 3.55 | Folded into the phase as a pluggable hook, but bundling injection/PII models is deferred — avoid becoming an LLM-ops product |
| 7 | NHI / delegation (RFC 8693) | 2.85 | High value but higher effort/lower feasibility; sequenced to the *next* phase once MCP RS + policy engine exist to build on |
| — | Semantic caching / multi-LLM routing | 2.50 | Off-identity: this is the LLM-ops bundle (Kong/Portkey/Cloudflare). Entering it dilutes the auth-proxy positioning. Separate strategic decision, not this phase |

---

## Recommended next command

```
/kbd-new-phase agent-authz-control-plane --seed .kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md
```

This seeds the next KBD phase (assess stage skipped) with this brief. Suggested phase goals map to the build order: (1) budget+rate-limit enforcement, (2) MCP OAuth 2.1 resource-server, (3) embedded policy engine + per-tool authz, (4) HITL approval, (5) web config UI, (6) guardrail-hook + audit trail.
