# Goals — agent-governance-completeness-and-policy-authoring

_Seeded from `agent-gateway-budget-and-policy-operability/reflection.md` →
"Recommended Next Phase". The prior phase made the agent-governance controls
**hard to misconfigure**; this phase makes them **complete in coverage** — it pays
down the two structural debts that phase deliberately deferred (both about making
governance comprehensive rather than boot-YAML-/config-only bounded), and adds the
deferred authoring surface._

## Phase Goal

Close the coverage gaps in agent governance so the guarantees are **comprehensive,
not scoped to the easy path**: lint and govern DB-sourced routes (not just YAML),
merge config tool-scope sugar into the DB-backed policy engine (so it enforces
instead of refusing-start), and give operators an admin-UI affordance to author
agent tool-scopes. Still **authorization-first**; still **federate any JWKS-capable
IdM (Ory reference), never an IdP**; the LLM-ops bundle stays out of scope.

**Seeded from:** `agent-gateway-budget-and-policy-operability` reflection ·
**Criteria profile:** effort-impact

## Goals (build order — dependency-aware)

1. **Lint + govern DB-sourced routes** *(prior-phase debt #1 — BUILD FIRST; the
   reusable helper already exists).* Run `agent_governance_lint_routes` on the
   **merged (YAML + DB) route set** at load and on every hot-reload, so
   `strict_agent_governance` is a comprehensive guarantee rather than a
   boot-YAML-scoped one. Today `agent_governance_lint()` walks `self.routes` (YAML)
   only; routes merged from `gate_routes` under `database.override_yaml` and via the
   LISTEN/NOTIFY reload path escape the lint entirely. (HIGH — closes the sharpest
   remaining "silent under-application" edge left open last phase.)

2. **Merge config `agent_tool_policies` into the DB-backed engine** *(prior-phase
   debt #2).* Compile the config tool-scope sugar into the live Cedar engine
   **alongside** DB-stored policies (parse-before-swap, fail-closed) instead of the
   current "refuse-start when sugar + DB coexist" guard — then **lift that guard**.
   The sugar should enforce in the common DB deployment, not only in the config-only
   deployment. Preserve deny-wins, glob support, injection-safety, and the
   write-time validation gate. (HIGH — closes debt #2; unblocks the sugar for the
   deployment operators actually run.)

3. **Admin-UI policy builder for agent tool-scopes** *(the deferred G3 sub-scope).*
   A Policies-tab affordance to author agent allow/deny tool-scopes over the
   now-merged sugar — the ergonomic front-end that compiles to the same validated
   Cedar, surfaced in the existing admin web UI rather than only via config file.
   (MEDIUM.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt compression,
  multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- **Merged-route lint timing:** lint the merged set once at load AND on every
  hot-reload NOTIFY, or gate the reload itself (reject a DB route that fails the
  lint under strict mode)? How does refuse-vs-warn interact with a *running*
  gateway when a bad DB route arrives via hot-reload — you can't `bail!` a live
  process. (Crux of goal 1 — assess the reload path's error model.)
- **Sugar↔DB merge semantics:** when both a config sugar policy and a DB policy
  name the same agent/tool, what wins? (Cedar `forbid` still overrides, but two
  `permit`s / a sugar-permit vs a DB-forbid need a defined precedence.) Namespace
  the sugar PolicyIds to avoid collisions with DB rows. (Crux of goal 2.)
- **Sugar as a DB source vs a parallel set:** does the merge write the compiled
  sugar into `gate_routes`/policy rows (single source of truth) or keep it a
  parallel in-memory overlay rebuilt on reload? (Affects hot-reload + the admin UI.)
- **Admin-UI surface:** does the builder edit the config sugar, write DB policy
  rows directly, or both? How does it reuse the existing Policies-tab + write-time
  validation? (Crux of goal 3.)

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] The governance lint covers DB-sourced routes: a merged (YAML+DB) route set is
      linted at load and on hot-reload; `strict_agent_governance` is comprehensive
      (test-proven with a DB-only under-governed route).
- [ ] Config `agent_tool_policies` are compiled into the DB-backed engine and
      enforced alongside DB policies (parse-before-swap, fail-closed); the
      "refuse-start when sugar+DB coexist" guard is lifted; deny-wins / glob /
      injection-safety / validation preserved (test-proven).
- [ ] Operators can author agent tool allow/deny scopes from the admin UI; the
      builder compiles to the same validated Cedar.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new lint/policy/reload path fail-safe (tested);
      separated security review on each policy/authz/reload-touching change.
