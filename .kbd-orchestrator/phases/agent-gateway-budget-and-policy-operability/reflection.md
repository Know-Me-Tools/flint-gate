# Reflection — agent-gateway-budget-and-policy-operability

_Phase closed 2026-07-07. Backend: openspec · Driver: kbd-apply · 3/3 changes
archived. Seeded from `agent-gateway-mcp-tool-governance` reflection; this phase
was the **operability layer** — make the prior phase's agent-governance controls
hard to misconfigure and complete in coverage, and close its recorded debt._

## Goal Achievement

| # | Goal | Verdict | Evidence |
|---|------|---------|----------|
| G1 | Config-validation lints for agent governance (WARN default / strict refuse-start) | **MET** | `agent_governance_lint[_routes]()` flags agent-reachable routes with non-agent budgets / no authorize hook / lifetime-agent budgets / unresolvable providers; WARN by default, `server.strict_agent_governance` → refuse-start. Change `add-agent-governance-lint`. |
| G2 | Symmetric exchange metrics + strict cross-replica rate-limit mode | **MET** | `flint_local_exchange_total{result}` meters the local-mint path (both modes observable); `oauth.rate_limit.require_shared_backend` refuses start when exposed non-loopback without a real shared limiter. Change `add-local-exchange-metric-strict-ratelimit`. |
| G3 | Cedar policy ergonomics for agent tool-scoping | **MET (config sugar); admin-UI DEFERRED** | `agent_tool_policies` compiles to validated Cedar `permit`/`forbid` on `call_tool`; deny-wins; glob support; injection-safe; fail-closed at load. Admin-UI policy builder deferred by design (decision-log). Change `add-agent-tool-scope-sugar`. |
| G4 | Fail-closed lifetime agent budgets | **MET (via config-refusal, as decided)** | Resolved by refusing `scope: agent` + `window: lifetime` at config-lint time (folded into G1's `LifetimeAgentBudget` finding) rather than threading outage state through the ledger — per the 2026-07-07 decision-log entry. |

**Goal completion: 4/4 MET** (G3's admin-UI sub-scope was an explicit, logged
deferral, not a miss; G4 was met by the config-refusal path chosen in Analyze).

### Success-criteria checklist (from goals.md)

- [x] Startup validation WARNs (and refuses under strict) for agent-reachable
      routes with non-agent budget / no tool policy — test-proven, fail-safe default.
- [x] `flint_local_exchange_total{result}` meters the local-mint path; strict
      cross-replica mode available + test-proven fail-closed.
- [x] Operators author agent tool allow/deny scopes without raw Cedar; sugar
      compiles to validated Cedar.
- [x] `scope: agent` + `window: lifetime` refused at config-validation time.
- [x] Workspace green (`check`/`clippy -D warnings`/`test --workspace`); new
      features covered; every new auth/budget/lint/policy path fail-safe + tested;
      separated security review on each of the three changes.

## Delivered Changes

1. **`add-agent-governance-lint`** (G1 + G4) — pure `agent_governance_lint()` over
   resolved route providers; `GovernanceReason::{NonAgentScopedBudget,
   NoAuthorizeHook, LifetimeAgentBudget, UnresolvableAuthProvider}`;
   `server.strict_agent_governance` startup posture. 9 lint tests.
2. **`add-local-exchange-metric-strict-ratelimit`** (G2) — `flint_local_exchange_total{result}`
   + `exchange()` `?`→outcome-arms restructure (fail-closed preserved);
   `oauth.rate_limit.require_shared_backend` folded into `oauth_exposure_posture`
   (feature-aware). 12 tests across both feature configs.
3. **`add-agent-tool-scope-sugar`** (G3) — `authz/sugar.rs` compiler:
   `agent_tool_policies` → validated Cedar; allowlist-charset injection safety;
   deny-wins; globs; refuse-start on invalid sugar or on sugar-alongside-DB. 14 tests.

Zero new dependencies (decision-log): all three build on in-tree `cedar-policy 4`,
`metrics`, the posture pattern, and the write-time validator.

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA | 3/3 |
| First-pass pass rate | 3/3 (100%) |
| Changes requiring refinement iteration | 0 (all passed; fixes applied pre-archive) |
| Security-review findings fixed before archive | 2 (both LOW, both in this session's own code) |

**First-pass** here means: no change was ever marked BLOCKED / sent back for a
second QA cycle. Every change passed its constraint gate and separated security
review on the first pass — but two reviews surfaced a LOW finding in code written
*this* phase that was fixed before archive (counted below, not as a re-cycle).

### Constraint violations

None. All three changes PASSED every blocking constraint (no secrets; admin bind
unaffected; no broken tests; config priority CLI>env>YAML untouched).

### Security-review findings (all fixed pre-archive)

- `add-local-exchange-metric-strict-ratelimit` **LOW-1** — `has_shared_ratelimit_backend()`
  doc claimed a compiled-out `redis-l2` build would refuse, but the config-only
  predicate would have `Enforce`d → made the predicate require `cfg!(feature =
  "redis-l2")` so it genuinely refuses; feature-matrix test added.
- `add-agent-tool-scope-sugar` **LOW-1** — sugar validated-but-silently-unenforced
  when a DB is attached → strengthened the `warn!` to a **refuse-start** (a `deny:`
  rule must never look active while ignored).

Recurring pattern (2/3 changes): a **fail-safe seam that was *advisory* where it
should have *refused*** — caught only by the separated reviewer, not the author.
Both were corrected toward refuse-start. **Lesson reinforced:** the security review
is where "advisory vs. refuse" gets adjudicated; the author consistently defaults
one notch too permissive.

## Technical Debt Introduced

1. **DB-route governance lint gap** (from G1, carried from change 1) — the lint
   walks YAML routes only; DB-sourced routes (`database.override_yaml` / hot-reload)
   are unlinted. Mitigated: `agent_governance_lint_routes(&[..])` is reusable so a
   follow-up can lint the merged/DB set in one line; documented + tracked.
2. **Config-sugar engine-merge** (from G3) — `agent_tool_policies` are validated
   at startup and enforced ONLY in the no-DB (config-only) deployment; with a DB
   they refuse-start rather than merge. The merge into the DB-backed engine
   (+ the deferred admin-UI policy builder) is the natural follow-up.
3. **Validator is a parse gate, not a type gate** (G3, reviewer LOW-2, accepted) —
   sugar validation checks Cedar parseability, not schema-type conformance;
   consistent with the engine's optional-schema model, safe given the charset
   allowlist. Revisit if/when a mandatory schema lands.

## Lessons Captured

- **Reuse the pipeline's own resolution, don't invent a parallel model.** "Agent-
  reachable" reused `route.auth ?? site.default_auth → provider variant` exactly as
  `pipeline.rs` resolves it — no second reachability notion to drift.
- **Config-time refusal beats runtime plumbing for corner cases.** G4's lifetime-
  agent-budget fail-closed was cheaper and more visible as a one-path config lint
  than threading `Unavailable` through the ledger.
- **`?`→outcome-arms is behavior-preserving *if* every arm returns the original
  error** — the reviewer confirmed the restructure changed only instrumentation,
  not fail-closed outcomes. Worth the explicit per-arm metric.
- **Injection-safety by allowlist-charset-before-interpolation** is simpler and
  more auditable than escaping when compiling untrusted strings into a policy DSL.
- **Two disk/environment blips** (ENOSPC during linking) were correctly diagnosed
  as environment, not code — cleaning stale `target/debug/incremental` restored green.

## Recommended Next Phase

**`agent-governance-completeness-and-policy-authoring`** (working title) — pay down
the two structural debts this phase deliberately deferred, both of which are about
making the governance *complete* rather than *bounded*:

1. **Lint + govern DB-sourced routes** — run `agent_governance_lint_routes` on the
   merged (YAML + DB) route set at load and on every hot-reload, so `strict_agent_governance`
   is a comprehensive guarantee, not a boot-YAML-scoped one. (HIGH — closes debt #1;
   the reusable helper already exists.)
2. **Merge config `agent_tool_policies` into the DB-backed engine** — so the sugar
   enforces alongside DB policies instead of refusing-start when a DB is present,
   then lift the refuse-start guard. (HIGH — closes debt #2; unblocks the sugar for
   the common DB deployment.)
3. **Admin-UI policy builder** for agent tool-scopes (the deferred G3 sub-scope) —
   a Policies-tab affordance over the now-merged sugar. (MEDIUM.)

Still authorization-first; still federate-any-JWKS-IdM (Ory reference), never an
IdP; the LLM-ops bundle stays out of scope. **Criteria profile:** effort-impact —
the two HIGH items are small, well-scoped follow-throughs on helpers/patterns this
phase already built.
