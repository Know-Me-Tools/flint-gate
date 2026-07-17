# Reflection — agent-gateway-mcp-tool-governance

_Phase closed: 2026-07-07 · Backend: openspec · Driver: kbd-apply_
_Seeded from: `agent-gateway-exposure-operability/reflection.md`_

## Phase Goal (restated)

Deliver the **MCP-era agent-gateway** value the project exists for: govern **what
tools an agent may call and how much it may spend**, on the now-solid
identity/authz/exposure foundation. Still authorization-first; still **federate
any JWKS-capable IdM (Ory reference), never an IdP**; LLM-ops bundle out of scope.

## Goal Achievement

| Goal | Status | Evidence |
| --- | --- | --- |
| **G1 — Per-tool-call authz + delegate classification** | ✅ MET | `add-agent-delegate-classification`: the per-tool-call authz engine (`authorize_tool_call`, Cedar `call_tool`, `list_tools` filter, audit) was **already built** — Assess reframed the goal to classification + hardening. `act`-based Agent classification confirmed IdM-agnostic (Hydra-delegate included); hardened to require a well-formed `act` (RFC 8693 §4.1, `sub` required). **Fixed a HIGH-class forged-`flint_kind` escalation** found mid-change (below). |
| **G2 — Runtime cross-replica agent budgets** | ✅ MET | `add-agent-budget-scope`: added `BudgetScope::Agent` (distinct `flint:budget:agent:…` key, no collision), flowing through the existing Redis `incr_budget`/`get_budget`. **Closed the fail-OPEN seam**: `resolve_budget_usage` returns `BudgetUsage::{Known, Unavailable}` (was `0`-on-error = silent allow); on outage, Agent → Deny, User/Team → degrade. Hardened with a principal-kind **defense-in-depth** override after review. |
| **G3 — MCP tool-call observability** | ✅ MET | `add-tool-authz-metrics`: `flint_tool_authz_total{decision}` at the per-tool-call authz funnel + `flint_agent_budget_denied_total` at the agent-budget block points, on the admin `/metrics`. Decision-only `&'static str` labels (tool name stays in the DB audit — cardinality/leak-tested). |
| **G4 — (Optional) operability edge** | ⚠️ PARTIAL / deferred | The G4-stretch `flint_local_exchange_total` was **deferred** (documented) — the local-mint path is `?`-propagation, not worth restructuring a fail-closed path late in the phase for a low-value symmetric counter. The strict-rate-limit-mode edge was not undertaken. |

**3/4 goals MET; G4 was explicitly optional and partially deferred with a recorded
rationale.** All core-goal (G1–G3) success criteria are satisfied and test-proven.

## Delivered Changes

| # | Change | Goal | Tasks | Status |
| --- | --- | --- | --- | --- |
| 1 | `add-agent-delegate-classification` | G1 | 5/5 | archived |
| 2 | `add-agent-budget-scope` | G2 | 5/5 | archived |
| 3 | `add-tool-authz-metrics` | G3 | 5/5 | archived |

Build order followed the plan (classification → budget → metrics). Verification
gate met per change: `cargo clippy --workspace --all-targets -- -D warnings`
clean, `cargo test --workspace` green (413 → **420** core tests, +7 net; the
redis-l2-off build was also checked). **Zero new dependencies** — the whole phase
reused cedar-policy 4, the Redis budget counters, the `metrics` surface, and
`BackendUnavailablePosture` (research-validated: embedded Cedar ≈ an external PDP).

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA | 3/3 (100%) |
| First-pass pass rate | 3/3 (100%) — all PASS, none BLOCKED |
| Changes requiring refinement iteration | 0 (all review fixes applied inline, pre-archive) |
| Blocking-constraint violations | 0 across all 3 changes |
| Security findings surviving to archive | 0 |

### No recurring constraint violation

`no-secrets`, `admin-4457-not-public`, `no-broken-tests`, `config-priority` all
PASS in every log. Nothing failed once.

### Security findings caught & remediated *before* archive

- **G1 — HIGH-class privilege escalation (forged `flint_kind`).** All three
  federated authenticators (jwt, mcp, **kratos**) copied untrusted upstream
  `flint_kind` into the identity, which `derived_kind` trusts — a federated IdP or
  self-service Kratos user could forge `flint_kind: agent`/`service` and escalate
  to a non-human principal. An existing test even *codified* the vuln. Fixed:
  `flint_kind` stripped on all three paths; test replaced. Separated review
  (re-derived from `git diff`) confirmed the fix + two LOWs (Service re-entry
  asymmetry, `act` structural validation) both remediated.
- **G2 — MEDIUM scope-authority gap.** Fail-closed was bound to operator-declared
  `config.scope`, so a delegated agent on a `scope: user` route silently degraded
  on outage. Fixed with a principal-kind defense-in-depth: `outage_must_deny`
  denies when the budget is agent-scoped **OR** the actual principal is an Agent.
- **G3 — self-check PASS.** `record_tool_authz(&'static str)` structurally forbids
  a runtime tool-name label; admin-only surface; tool-name-leak tested.

## Technical Debt Introduced

1. **G4 `flint_local_exchange_total` deferred** — the gateway-local mint path is
   unmetered (only the delegate path is). Documented; a future change should meter
   it symmetrically, ideally by restructuring the `?`-propagation into outcome arms.
2. **Lifetime + Agent budgets are best-effort** — the fail-closed posture is scoped
   to *windowed* budgets; a `lifetime` agent budget still reads best-effort
   (allow-on-error). Documented; an operator wanting a fail-closed agent budget
   must use a fixed window.
3. **Agent-budget scope is operator-declared** — an agent's spend is only counted
   under the Agent budget when a route declares `scope: agent`. The
   defense-in-depth deny covers the *outage* case, but a mis-scoped route still
   *accounts* agent spend in the user keyspace. A startup lint (warn when an
   agent-reachable route has a non-agent-scoped budget) is the follow-up.
4. **Strict cross-replica rate-limit mode + local-mint metric** — carried from the
   prior phase, still open.

## Lessons Captured (knowledge base)

- **"Already built" is now a *recurring* Assess outcome — audit before assuming a
  gap.** For the third phase running, the seeded goal over-estimated the build:
  G1's tool-authz engine + `list_tools` filter were done; G2's budget primitives
  were wired. Grepping for the *actual* code (not the plan's assumption) reframed
  each phase into wire/harden/classify. Budget a real Assess pass against source.
- **Hardening a trust boundary means auditing every entry point to it, not just
  the one in the diff.** The `flint_kind` strip had to cover jwt AND mcp AND
  kratos — the escalation lived in the path (Kratos) the change didn't originally
  touch. A doc comment asserting a trust property ("only gateway-minted tokens
  carry it") is worthless until *enforced* at every boundary.
- **A security control's *reach* is as important as its *correctness*.** G2's
  fail-closed logic was correct but bound to config; the review's MEDIUM was that
  it didn't *reach* a mis-scoped agent. Binding the control to the actual
  principal (defense in depth), not just the operator's declaration, closed it.
- **External research can validate a design as much as inform one.** Firecrawl on
  the MCP authz spec + Cerbos's MCP-authz architecture independently described
  flint-gate's existing embedded-Cedar design — which turned "should we adopt an
  external PDP?" into a confident "no, it's already the equivalent," saving a
  dependency and a rebuild.
- **Separated security review keeps finding what self-review misses** — 1 HIGH + 1
  MEDIUM + several LOWs across 3 changes, all fixed before archive, zero surviving.

## Recommended Next Phase

**`agent-gateway-budget-and-policy-operability`** — the agent-gateway core
(tool policy + budgets + observability) now exists; the next phase makes it
**operable and self-defending against misconfiguration**, closing this phase's
debt:

1. **Config-validation lints for agent governance** *(closes debt #3)* — a startup
   lint that WARNs (or refuses) when an agent-reachable route has a non-agent-scoped
   budget or no tool policy, so the governance controls can't be silently
   under-applied. *Do first — it turns "operator must remember" into "the gateway
   tells you."*
2. **Symmetric exchange metrics + strict rate-limit mode** *(closes debt #1/#4)* —
   meter the local-mint path (restructure its `?`-propagation), add the deferred
   strict cross-replica token-endpoint `deny` posture.
3. **Cedar policy ergonomics for agent tool-scoping** *(the deferred G1 ergonomics)*
   — a higher-level "agent X may call tools [...]" authoring affordance + admin-UI
   support, on top of the raw Cedar + write-time validation that exists.
4. **Fail-closed lifetime agent budgets** *(closes debt #2)* — extend the
   fail-closed posture to lifetime-windowed agent budgets, or refuse the
   combination at config time.

Stay authorization-first; federate any JWKS IdM, never an IdP; LLM-ops bundle out
of scope. This phase built the agent-gateway controls; the next makes them **hard
to misconfigure and complete in their coverage** — the operability layer for
agent governance.
