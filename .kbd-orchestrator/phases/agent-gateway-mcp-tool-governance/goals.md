# Goals — agent-gateway-mcp-tool-governance

_Seeded from `agent-gateway-exposure-operability/reflection.md` → "Recommended
Next Phase". The prior three phases built the identity/authz control plane, made
it safe to expose, and made that exposure operable + observable. This phase
delivers the **MCP-era agent-gateway** value the project exists for: governing
**what tools an agent may call and how much it may spend**, on that now-solid
identity foundation._

## Phase Goal

Turn flint-gate into a credible **agent gateway**: make per-tool-call
authorization first-class (tool scoping per agent identity), enforce agent
**budgets at runtime** across replicas, and make agent tool-call behavior
**observable**. Close the delegate-token budget-bypass debt from the prior phase.
Still **authorization-first**; still **federate any JWKS-capable IdM (Ory
reference), never an IdP**; the LLM-ops bundle stays out of scope until the
tool-governance core is complete.

**Seeded from:** `agent-gateway-exposure-operability` reflection ·
**Criteria profile:** effort-impact

## Goals (build order — dependency-aware)

1. **Per-tool-call authorization + Cedar policy ergonomics** *(the core
   agent-gateway differentiator — BUILD FIRST)*. Build on the existing embedded
   Cedar engine + per-tool-call authz to make agent tool-scoping first-class:
   tool allow/deny lists per agent identity, resource-scoped grants
   (`Agent::"x"` may call `Tool::"deploy"` on `Resource::"y"`), and ergonomic
   policy authoring/validation. Resolve the **delegate-classification gap** from
   last phase — either a Hydra-side claim mapper that stamps an agent marker, or
   an explicit gateway policy path for delegate-mode tokens — so a delegated
   agent token is governed, not unclassified. (HIGH — this is why the project
   exists.)

2. **Runtime agent budget enforcement (cross-replica)** *(prior-phase debt #1/#2)*.
   Wire the windowed token budgets to the **shared Redis counters** (proven for
   rate-limiting this phase) so agent spend is cross-replica-accurate, not
   per-replica. Close the delegate-token budget-bypass: a delegate-mode token must
   still be subject to agent budget (via goal 1's classification fix). Fail-closed
   on the budget backend per the established outage posture. (HIGH — makes "agent
   budgets" a real runtime guarantee, not a config.)

3. **MCP tool-call observability** *(build on the metrics surface from last phase)*.
   Extend the `metrics` surface to per-tool authz **decisions** (allow/deny by
   tool + agent) and **budget consumption**, so operators see agent behavior — not
   just delegate volume. Keep `/metrics` admin-port-only; labels stay bounded /
   `&'static`-safe. (MEDIUM.)

4. **(Optional) Deferred operability edge** *(prior-phase debt #1/#2)*. A strict
   cross-replica rate-limit mode (token-endpoint `deny` posture, no per-replica
   degrade) and a symmetric metric for the **local-mint** exchange path so both
   exchange modes are observable. (LOW — polish; do only if goals 1–3 land with
   budget to spare.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt
  compression, multimodal, prompt versioning) — remains off until tool-governance
  is complete.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- **Delegate-classification fix shape:** Hydra-side claim mapper (keeps the
  gateway a pure verifier) vs. an explicit gateway policy for delegate-mode
  tokens. Which honors "federate, never an IdP" while still governing delegated
  agents? (Decide in Analyze — it gates goal 2's budget-bypass close.)
- **Tool identity model:** are MCP tools first-class Cedar `Resource`/`Action`
  entities, and how are tool names namespaced per upstream? (Assess the existing
  per-tool-call authz + Cedar schema first — this may be partly built, like the
  rate-limiter was.)
- **Budget window semantics for tools:** per-token vs per-call vs cost-weighted;
  reuse the `BudgetWindow`/`incr_budget` primitives already present.

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] An agent identity can be scoped to an allow/deny set of tools via Cedar
      policy; an out-of-scope tool call is denied (test-proven, fail-closed).
- [ ] Delegate-mode tokens are classified/governed (no longer bypass agent
      policy + budget); the chosen mechanism honors federate-never-an-IdP.
- [ ] Agent token/spend budgets are enforced **cross-replica** via the shared
      Redis counters; over-budget denies; a budget-backend outage fails closed.
- [ ] Per-tool authz decisions + budget consumption are observable on the admin
      `/metrics` surface, with bounded/static labels.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new authz/budget path fail-closed (tested);
      separated security review on each auth/budget-touching change.
