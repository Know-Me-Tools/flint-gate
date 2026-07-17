# Analysis — agent-gateway-budget-and-policy-operability

_Analyzed: 2026-07-07 · Mode: stack-specified (Rust / cedar-policy 4 / axum / Ory)_
_Research: local source inspection + one firecrawl pass (config-lint severity
convention — no source beat flint-gate's own admin/oauth precedent). Within budget._

## Standing constraint (load-bearing)

Support any IdM with a JWKS pathway; **Ory Kratos/Hydra is the standard**;
**federate, never become an IdP.** Governs the agent-reachable detection (the
JWKS-provider signal) and keeps G3 sugar compiling to Cedar the gateway validates,
not a new policy authority.

## Landscape summary

**Every goal is build-with-existing — zero new dependencies.** The lint,
strict-mode posture, local-mint metric, Cedar codegen, and lifetime-refusal all
build on in-tree primitives (`cedar-policy 4`, `metrics`, the `*_posture()→bail!`
pattern, the Cedar write-time validator, the `record_delegate` metric pattern).
No library-adoption decision exists this phase.

## Build-vs-adopt calls (all BUILD, no new dep)

### G1 — Agent-governance config lint · **BUILD — reuse posture pattern + route resolution**

`GateConfig::agent_governance_lint() -> Vec<GovernanceFinding>` walking each route,
resolving its effective auth provider **exactly as the pipeline does**
(`matched_route.config.auth` ?? `site.default_auth` → `auth_providers` map →
variant, per `pipeline.rs:139-143`), and flagging:
- an **agent-reachable** route (provider is `Jwt`|`Mcp`) whose `MaxTokenBudget`
  hook is non-agent-scoped;
- an agent-reachable route with **no `Authorize` hook** at all.

**Open Q1 — severity default — DECIDED: WARN by default, opt-in strict → refuse-start.**
Rationale, grounded in flint-gate's OWN precedent (stronger than the generic
API-security articles the web search surfaced): `admin_auth_posture` /
`oauth_exposure_posture` **allow the loose case by default** (loopback) and only
`RefuseStart` when the surface is actually exposed. Mirror that: a governance
finding **WARNs** by default (don't break existing loose configs on upgrade), with
a `server.strict_agent_governance: bool` (off by default) that promotes findings to
a startup `bail!`. Fail-safe *escalation path*, non-breaking default.

**Open Q2 — agent-reachable precision — DECIDED: provider-type via the route→site
resolution.** A route is agent-reachable iff its **resolved** provider (route.auth,
else site.default_auth) is `Jwt` or `Mcp`. Kratos=human, ApiKey=Service (already
`kind: Service` explicitly), Anonymous=none. This is the least-surprising signal
and reuses the pipeline's own resolution — no new "reachability" model.

### G4 — Fail-closed lifetime agent budgets · **BUILD — fold into G1 (refuse at config)**

**Open Q4 — fail-closed lifetime read vs refuse-at-config — DECIDED: refuse at
config (fold into G1's lint).** Making the lifetime lookup return `Unavailable`
would thread outage state through the ledger path for a corner case; instead the
G1 lint flags `scope: agent` + `window: lifetime` as a governance finding
(WARN/strict-refuse), consistent with the already-documented "fail-closed agent
budgets require a fixed window." Smaller, one code path, and it makes the
constraint visible to the operator. *(DECIDED.)*

### G2 — Local-mint metric + strict cross-replica mode · **BUILD — reuse metric pattern + posture**

- **Local-mint metric:** add `flint_local_exchange_total{result}` via the
  `record_delegate` pattern (`&'static str` labels). The local `exchange()` branch
  is `?`-propagation (verify→downscope→mint); restructure into outcome arms
  (`success` / `deny_verify` / `deny_downscope` / `mint_failed`) — a mechanical
  refactor of a fail-closed path with existing unit coverage to guard it.
- **Strict cross-replica rate-limit mode — DECIDED: a new startup posture, not a
  runtime flag (Open Q, resolved).** `on_backend_unavailable` only governs a
  mid-request Redis *error*; it does NOT cover "no shared cross-replica limiter
  configured at all" (redis-l2 off / no `cache.l2` → silent per-replica governor
  fallback). Add `oauth.rate_limit.require_shared_backend: bool` (off by default):
  when true, **refuse to start** if the OAuth surface is exposed non-loopback
  without a shared Redis limiter — folds into the existing `oauth_exposure_posture`.
  This makes "I need cross-replica-accurate limits" an enforced invariant, not a
  hope.

### G3 — Cedar tool-scope sugar · **BUILD — compile to Cedar, validate with the existing validator**

Add a per-agent config block — e.g. `agent_tool_policies: [{ agent: "ci-bot",
allow: ["deploy"], deny: ["delete_*"] }]` — that **compiles to Cedar `permit`/
`forbid` on `Action::"call_tool"` + `Route::"<tool>"`**, then runs through the
existing `authz/validator.rs` write-time validation before load. No new policy
engine; the sugar is a front-end over the Cedar the engine already runs.

**Open Q3 — sugar shape + UI scope — DECIDED: config-block sugar this phase; admin-UI
builder DEFERRED.** The config sugar delivers the ergonomics win with the least
surface; a UI policy-builder is a larger, separable effort. Ship the sugar +
validation now; note the UI as follow-up. *(RECOMMEND — confirm in Spec.)*

## Net effect on scope

**Zero new dependencies.** One new lint fn + severity flag (G1+G4), one metric +
one refactor + one startup posture (G2), one config-sugar→Cedar compiler (G3).
Confidence **high** (all local-source-confirmed; the severity default is anchored
in the repo's own posture precedent). The phase is **operability + ergonomics over
existing controls**, not new infrastructure.

## Open questions — resolved here

1. Lint severity → **WARN default + opt-in strict(refuse-start)** per the admin/oauth precedent. *(DECIDED)*
2. Agent-reachable → **resolved provider is Jwt|Mcp** (route.auth ?? site.default_auth). *(DECIDED)*
3. G3 sugar → **config-block sugar this phase; admin-UI deferred.** *(RECOMMEND — confirm in Spec)*
4. G4 → **refuse `scope:agent`+`window:lifetime` at config (fold into G1's lint).** *(DECIDED)*

No contested stack, no new library adoption — no elicitation required.
