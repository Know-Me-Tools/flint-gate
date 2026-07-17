# Goals — agent-gateway-budget-and-policy-operability

_Seeded from `agent-gateway-mcp-tool-governance/reflection.md` → "Recommended Next
Phase". The prior phase built the agent-gateway **controls** (tool policy, agent
budgets, observability); this phase makes them **hard to misconfigure and complete
in their coverage** — the operability layer for agent governance — and closes that
phase's recorded debt._

## Phase Goal

Make the agent-gateway governance controls **self-defending against
misconfiguration and complete in coverage**: catch under-applied controls at
startup, meter both exchange modes, give operators an ergonomic way to author
agent tool-scopes, and extend fail-closed to the last budget window that lacks it.
Still **authorization-first**; still **federate any JWKS-capable IdM (Ory
reference), never an IdP**; the LLM-ops bundle stays out of scope.

**Seeded from:** `agent-gateway-mcp-tool-governance` reflection ·
**Criteria profile:** effort-impact

## Goals (build order — dependency-aware)

1. **Config-validation lints for agent governance** *(prior-phase debt #3 —
   BUILD FIRST; turns "operator must remember" into "the gateway tells you")*. A
   startup lint that **WARNs (or, on an opt-in strict setting, refuses to start)**
   when an **agent-reachable route** has a budget left at a non-agent scope, or has
   no per-tool authz policy. Agent spend can currently be silently accounted in the
   user keyspace when a route forgets `scope: agent`; the outage fail-closed covers
   the deny case but not the *accounting* gap. Surface it loudly at startup.
   (HIGH — closes the sharpest "silent under-application" edge.)

2. **Symmetric exchange metrics + strict cross-replica rate-limit mode**
   *(prior-phase debt #1 + carried debt #4)*. Meter the **gateway-local mint** path
   (`flint_local_exchange_total{result}`) so both exchange modes are observable —
   restructure its `?`-propagation into outcome arms. Add the deferred **strict
   cross-replica rate-limit mode**: a token-endpoint `deny` posture (no per-replica
   governor degrade) for operators who need a hard cross-replica guarantee.
   (MEDIUM.)

3. **Cedar policy ergonomics for agent tool-scoping** *(the deferred G1
   ergonomics)*. A higher-level "**agent X may call tools [...]**" authoring
   affordance (config sugar and/or admin-UI support) that compiles to the raw
   Cedar the engine already validates at write time — so operators don't hand-write
   `permit(principal == Agent::"x", action == Action::"call_tool", …)` per tool.
   (MEDIUM.)

4. **Fail-closed lifetime agent budgets** *(prior-phase debt #2)*. Extend the
   fail-closed outage posture to **lifetime-windowed** agent budgets (currently
   best-effort / allow-on-error), OR refuse the `scope: agent` + `window: lifetime`
   combination at config-validation time (ties to goal 1). (LOW — the last
   fail-open budget corner.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt compression,
  multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- **Lint severity model:** WARN-by-default with an opt-in `strict` (refuse-start)
  posture, vs. refuse-start by default. What's the least-surprising default that
  doesn't break existing loose configs? (Decide in Assess — mirror the
  admin/oauth exposure-posture precedent.)
- **"Agent-reachable route" detection:** how does the lint know a route can be hit
  by an agent principal — by auth provider, by policy, or by an explicit route
  annotation? (This is the crux of goal 1; assess the route/auth config model.)
- **Ergonomics surface:** config sugar (a `tools: [allow: [...], deny: [...]]`
  block that compiles to Cedar) vs. an admin-UI policy builder vs. both. Which
  fits the existing Policies tab + write-time validation?
- **Local-mint metering:** restructure the `?`-propagation vs. a lighter wrapper —
  is per-outcome granularity worth the refactor of a fail-closed path?

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] Startup validation WARNs (and refuses under a strict setting) when an
      agent-reachable route has a non-agent-scoped budget or no tool policy
      (test-proven, fail-safe default).
- [ ] `flint_local_exchange_total{result}` meters the gateway-local mint path
      (both exchange modes observable); a strict cross-replica rate-limit `deny`
      mode is available and test-proven fail-closed.
- [ ] Operators can author agent tool allow/deny scopes without hand-writing raw
      Cedar; the sugar compiles to validated Cedar.
- [ ] `scope: agent` + `window: lifetime` either fails closed on outage or is
      refused at config-validation time.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new auth/budget/lint path fail-safe (tested);
      separated security review on each auth/budget/policy-touching change.
