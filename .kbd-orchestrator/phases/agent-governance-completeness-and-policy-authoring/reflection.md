# Reflection — agent-governance-completeness-and-policy-authoring

_Phase closed 2026-07-08. Backend: openspec · Driver: kbd-apply · 3/3 changes
archived. Seeded from `agent-gateway-budget-and-policy-operability` reflection;
this phase paid down that phase's two deferred structural debts (make governance
**complete**, not just hard-to-misconfigure) and added the deferred authoring
surface._

## Goal Achievement

| # | Goal | Verdict | Evidence |
|---|------|---------|----------|
| G1 | Lint + govern DB-sourced routes | **MET** | `merge_routes` surfaces the merged (YAML+DB) set; startup lints it (bail-under-strict), hot-reload lints it (WARN; strict → reject-and-retain last-good, never terminate) + `flint_governance_reload_rejected_total`. Change `lint-db-sourced-routes`. |
| G2 | Merge config `agent_tool_policies` into the DB-backed engine | **MET** | Immutable sugar overlay on `AuthzEngine`, concatenated on every build/reload; refuse-start guard removed; deny-wins (Cedar forbid-overrides-permit) tested; reserved-id write-guard prevents collision. Change `merge-agent-tool-policies-into-engine`. |
| G3 | Admin-UI policy builder for agent tool-scopes | **MET** | `/tool-scopes` endpoint (structured-only `{agent,allow,deny}` → `compile_and_validate` → DB row) + Policies-tab builder UI. Change `admin-tool-scope-builder`. |

**Goal completion: 3/3 MET.** All three were the deferred debts/sub-scope this
phase existed to close; none slipped.

### Success-criteria checklist (from goals.md)

- [x] Governance lint covers DB-sourced routes (merged set at load + hot-reload);
      `strict_agent_governance` is comprehensive (DB-only under-governed route
      test-proven; reload is non-terminating).
- [x] Config `agent_tool_policies` compiled into the DB-backed engine, enforced
      alongside DB policies (parse-before-swap, fail-closed); refuse-start guard
      lifted; deny-wins / glob / injection-safety / validation preserved + tested.
- [x] Operators author agent tool allow/deny scopes from the admin UI; the builder
      compiles to the same validated Cedar.
- [x] Workspace green (`check`/`clippy -D warnings`/`test --workspace`); new
      features ≥80% covered; every new lint/policy/reload path fail-safe + tested;
      web build green; separated security review on each change.

## Delivered Changes

1. **`lint-db-sourced-routes`** (G1) — pure `merge_routes` + shared
   `from_config_with_routes`; merged-set lint at startup (`main.rs` 8b) and on
   hot-reload (`rebuild_router_from_db`, reject-and-retain under strict);
   `flint_governance_reload_rejected_total` metric. 7 tests.
2. **`merge-agent-tool-policies-into-engine`** (G2) — immutable `sugar` overlay on
   `AuthzEngine`; `concat_records` merges DB ++ sugar in `from_database_with_sugar`
   + both reload paths; refuse-start guard removed; reserved-id write-guard. 7 tests.
3. **`admin-tool-scope-builder`** (G3) — `/tool-scopes` admin endpoint (pure
   `compile_tool_scope`, structured-only, no raw-Cedar) + React Policies-tab builder
   (types, client fns, hooks, modal form). 5 backend tests + web build.

Zero new dependencies (decision-log): all three built on in-tree `cedar-policy 4`,
the lint helper, the sugar compiler, the merge-capable bundle, the admin CRUD
pattern, and the React/TanStack-Query/Tailwind admin kit.

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA | 3/3 |
| First-pass pass rate | 3/3 (100%) |
| Changes requiring a refinement re-cycle | 0 (all passed; fixes applied pre-archive) |
| Security-review findings fixed before archive | 3 (2 LOW + 1 MEDIUM, all in this session's own code) |

**First-pass** = no change was ever marked BLOCKED or sent back for a second QA
cycle. Each passed its constraint gate and separated security review on the first
pass; findings surfaced by the reviewer were fixed before archive (counted below,
not as a re-cycle).

### Constraint violations

None. All three PASSED every blocking constraint (no secrets; admin 4457 stays
private — the new `/tool-scopes` endpoint is admin-router only; no broken tests;
config priority CLI>env>YAML untouched).

### Security-review findings (all fixed pre-archive)

- `lint-db-sourced-routes` **LOW×2** — stale docstring (this change *was* the
  "follow-up" it referenced) → corrected; a strict reload-rejection was only
  log-grep-able → added `flint_governance_reload_rejected_total` (alertable).
- `merge-agent-tool-policies-into-engine` **MEDIUM** — `concat_records` documented a
  PolicyId-namespace-disjointness invariant it did not enforce; a privileged admin
  could store a policy with a `agent_tool_sugar::…` id and silently suppress a
  config `deny:` on the lenient reload path → fixed with a `SUGAR_ID_PREFIX`
  single-source const + a reserved-namespace **400-guard** at the admin/DB write
  boundary (invariant now true by construction).
- `admin-tool-scope-builder` — review PASS, **no findings** (the highest-risk
  change: the mandated API-boundary injection re-check confirmed an admin-API
  attacker cannot inject arbitrary Cedar).

**Recurring pattern (this phase):** the separated reviewer keeps finding the gap
between a **documented invariant and an enforced one** — last phase it was
"advisory where it should refuse"; this phase it was "a comment claims
id-disjointness / a follow-up landed" that the code didn't yet back. **Lesson
reinforced:** when a doc comment asserts a safety property, either enforce it in
code or write the enforcement as the same task — the review is where the unbacked
claim gets caught.

## Technical Debt Introduced

1. **Config-sugar hot-reload** — the sugar overlay is fixed at startup; editing
   `agent_tool_policies` in the config file at runtime needs a restart (documented).
   The DB-row path (config file → overlay) is not hot-reloadable, unlike the
   admin-UI path (DB rows, hot-reloadable). Minor: two authoring surfaces with
   different reload semantics.
2. **Tool-scope UI is create/replace, not a round-trip editor** — the builder
   posts `{agent,allow,deny}` and stores compiled Cedar; editing an existing scope
   re-authors from blank allow/deny (the stored form is the compiled text, not the
   structured lists). A structured round-trip (store the allow/deny alongside the
   compiled row) is a UX follow-up.
3. **`internal_error` leaks DB error strings** on the admin surface (pre-existing
   pattern, reviewer LOW) — acceptable on the authenticated loopback surface; revisit
   if the admin API is ever widened.

## Lessons Captured

- **Extract-then-reuse keeps a refactor provably behavior-preserving.** `merge_routes`
  / `from_config_with_routes` were byte-identical extractions (the reviewer diffed
  vs HEAD) — surfacing state for a new consumer without touching behavior.
- **A live process can't `bail!`.** The reload-time governance posture had to be
  reject-and-retain-last-good, not refuse-to-start — the fail-closed analog for a
  running gateway. Worth a metric so the silent retention is observable.
- **Put the overlay on the shared object, not the call sites.** Carrying `sugar` on
  `AuthzEngine` meant all three reload call sites inherited re-application for free —
  a single-point invariant beats wiring N sites.
- **Cedar's formal guarantees do real work.** `forbid-overrides-permit` +
  `skip-on-error` + `default-deny` (from the authoritative reference) let the merge
  skip custom precedence code entirely and stay fail-closed.
- **Reserved namespaces need a write-time guard, not a comment.** The MEDIUM finding.

## Recommended Next Phase

The agent-governance core is now complete and self-defending. Two credible
directions; recommend the operator pick per priority:

**Option A — `agent-approval-and-step-up-flows`** (governance depth). The authz
engine already models `RequireApproval` (`@require_approval` annotation → pause a
tool call) and there is an approval decision endpoint, but the end-to-end
human-in-the-loop flow (pause → surface an approval request over AG-UI/A2UI →
resume/deny) and a UI for pending approvals are not built out. This deepens the
"authorization-first" stance into interactive governance. (MEDIUM–HIGH.)

**Option B — `agent-governance-observability-and-ops`** (operability). A governance
dashboard over the metrics + audit trail this line of phases produced
(`flint_tool_authz_total`, `flint_agent_budget_denied_total`,
`flint_governance_reload_rejected_total`, the authz audit table): who/what is being
denied, budget burn-down, reload rejections — turning the raw signals into an
operator view. (MEDIUM.)

Small debt-paydown items (config-sugar hot-reload; tool-scope round-trip editor)
can ride along in whichever phase is chosen. Still authorization-first; still
federate-any-JWKS-IdM (Ory reference), never an IdP; LLM-ops bundle out of scope.
**Criteria profile:** effort-impact.
