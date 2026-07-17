# Analysis — agent-governance-completeness-and-policy-authoring

_Analyzed 2026-07-07. Mode: build-vs-adopt for three internal-wiring goals. The
Assess stage already established **zero external adoption surface** — every goal
extends in-tree primitives (`cedar-policy 4`, the lint helper, the sugar compiler,
the merge-capable bundle, the React/TanStack-Query admin kit). So this Analyze is
**design-decision resolution**, not library discovery: it confirms the two crux
architecture calls against the authoritative Cedar reference and records the
build plan._

## Research pipeline outcome

- **Tier 1 (gh search repos/code):** N/A — no framework/skeleton to adopt; the
  work is wiring existing in-repo functions (`agent_governance_lint_routes`,
  `compile_and_validate`, `CedarBundle::from_records`, the `/policies` CRUD +
  React admin patterns). Building anything net-new here would duplicate what the
  prior two phases already shipped.
- **Tier 2 (authoritative docs):** Cedar Policy Language Reference —
  **Security** page (`docs.cedarpolicy.com/other/security.html`) — consulted to
  confirm the guarantees the merge design leans on. Findings below.
- **Tier 3 (registries):** N/A — no new crate/npm candidate.
- **Tier 4 (broad web):** one confirmatory Firecrawl search (Cedar deny-wins
  semantics); credit-refund feedback attempted.

**Build-vs-adopt verdict: BUILD (wire), for all three goals. ZERO new
dependencies.** No `adopt`/`adapt` candidates — see `library-candidates.json`
(`build_required[]` only).

## Authoritative Cedar findings (Tier 2 — resolve G2 open questions)

From the Cedar Security reference (the semantics are formally modeled in Lean +
differential-tested, so these are hard guarantees, not conventions):

1. **`forbid-overrides-permit`** — "A single `forbid` policy evaluating to true
   results in a `Deny`." → **G2 precedence is provably safe under a merged
   PolicySet.** When sugar `PolicyRecord`s and DB rows are merged into ONE
   `PolicySet` (which `CedarBundle::from_records` already does), source does not
   matter: a DB `forbid` still overrides a sugar `permit` and vice-versa. There is
   no "which set wins" question at the deny level — deny always wins. The only
   design choice left is the **two-`permit`** case (sugar-permit + DB-permit → both
   allow; union semantics, which is the intended additive behavior).

2. **`skip-on-error` (default-deny)** — "An error in a policy results in that
   policy being ignored." + "default-deny." → **validates the lenient-merge reload
   path.** `from_records_lenient` dropping a bad row is exactly Cedar's own
   isolation model; a malformed merged row cannot open the gate (default-deny) and
   cannot corrupt sibling policies ("evaluation of one policy can't affect
   another"). This is the authoritative basis for G2's parse-before-swap +
   retain-last-good on reload.

3. **"avoid string concatenation … an attacker could achieve Cedar code
   injection"** — the reference explicitly demonstrates the
   `"principal,action,resource); //"` breakout. → **G3 constraint (NEW, important):**
   the sugar compiler builds Cedar via `format!` from operator strings; today that
   input arrives via a trusted YAML file, but a G3 admin-UI endpoint makes it
   **attacker-adjacent** (an authenticated-but-hostile admin, or a compromised
   admin session). The existing allowlist-charset defense (last phase's security
   review) MUST be preserved and re-reviewed at the API boundary — the endpoint
   must run the same `compile_and_validate` gate, never a looser path. Cedar's own
   guidance ("use templates, not concatenation") is a note that the allowlist is
   the accepted mitigation when concatenation is unavoidable.

## Design decisions (crux open questions → resolved)

### D1 (G1) — Reload-time governance error model
**Decision:** lint the merged (YAML+DB) route set at BOTH points, with
**stage-appropriate severity**: startup keeps the existing `bail!`-under-strict
(pre-serve, safe); **hot-reload WARNs always and, under `strict_agent_governance`,
REJECTS the offending DB route and retains the last-good router** (never
terminates a live process). · **Provenance:** Assess finding (reload path has no
`bail!`; `rebuild_router_from_db` at `cache/mod.rs:398` is best-effort
rebuild-or-retain) + the axum/tokio reality that you cannot exit a serving
process on a background NOTIFY. · **Rationale:** matches the codebase's own reload
discipline (bad route → WARN-skip; load failure → unchanged) and the Cedar
default-deny/skip-on-error posture — reject-and-retain is the fail-closed analog
of `bail!` for a running process.

### D2 (G1) — Surfacing the merged route set for the lint
**Decision:** have `from_config_and_db_routes` (or a sibling) **expose the merged
`Vec<RouteConfig>`** it already builds at `router.rs:128` (currently dropped), and
lint that slice via the existing `agent_governance_lint_routes`. Prefer surfacing
over recomputing the merge (single source of truth; no drift). · **Provenance:**
Assess (`router.rs:128` builds then discards the merged set). · **Rationale:**
the helper already takes `&[RouteConfig]`; only the plumbing to hand it the merged
slice is missing.

### D3 (G2) — Sugar reload persistence
**Decision:** **store the validated `sugar_policies: Vec<PolicyRecord>` on the
`AuthzEngine`** (an immutable overlay set fixed at startup from config) and
**concatenate `DB records ++ sugar` in every build/reload path** (`from_database`,
`reload_from_database`, and the admin-CRUD reload). Keep config the source of
truth for sugar (in-memory overlay), NOT written into `authz_policies` rows. ·
**Provenance:** Assess (`reload_from_database` is DB-only → drops sugar on first
reload; `cache/mod.rs:372`, `admin/mod.rs:727`). · **Rationale:** overlay is the
lower-risk default — it keeps "who owns this policy" clear (config vs DB), avoids a
migration writing compiled Cedar into rows, and the `CedarBundle` merge primitive
already accepts a combined slice. The sugar set is immutable for the process
lifetime (config hot-reload of sugar is a separable follow-up), so a single stored
`Arc<Vec<PolicyRecord>>` on the engine suffices.

### D4 (G2) — Precedence + guard removal
**Decision:** rely on Cedar `forbid-overrides-permit` for cross-source conflicts
(no custom precedence code); **remove the `db.is_some() && !sugar.is_empty()`
refuse-start guard** (`main.rs:306-315`) once D3 lands. Add behavioral tests for
sugar-permit vs DB-forbid (deny), DB-permit vs sugar-forbid (deny), two-permit
(allow). · **Provenance:** Tier-2 Cedar finding #1. · **Rationale:** deny-wins is
a formally-verified Cedar guarantee; encoding our own precedence would be
redundant and riskier.

### D5 (G3) — Admin-UI builder write target
**Decision:** add a **new admin endpoint** that accepts
`{agent, allow[], deny[]}`, runs `compile_and_validate` (same 400-gate as
`/policies`), and persists the entries as the config-sugar overlay source (aligned
with D3's overlay model) — surfaced by a Policies-tab builder form following the
**AgentIdentities.tsx** (modal builder + RQ hooks) and **Routes.tsx** (nested
structured config) patterns. The endpoint MUST route operator input through the
same allowlist-charset `compile_and_validate`, never a raw-Cedar bypass. ·
**Provenance:** Tier-2 finding #3 (injection) + Assess (no admin surface today;
overlay model from D3). · **Rationale:** reuses the proven compiler + validation
gate; keeps the UI-authored scopes in the same enforced set G2 establishes;
depends on G2 (build order G1→G2→G3).

## Risks / watch-items

- **G2 overlay vs config-hot-reload of sugar:** the overlay is fixed at startup;
  changing `agent_tool_policies` in the config file at runtime won't take effect
  until restart (config-file reload rebuilds the router, not the sugar overlay).
  Acceptable for this phase (document it); a sugar-hot-reload is a follow-up.
- **G3 injection at the API boundary:** the single most important security item —
  the endpoint must not offer a raw-Cedar path for tool-scopes; it compiles from
  the structured `{agent,allow,deny}` only. Separated security review required.
- **G1 reload strictness surprise:** reject-and-retain under strict mode means a
  hot-reloaded bad DB route is silently NOT applied (last-good retained) — must be
  loudly logged so an operator isn't confused why their route didn't take.

## Open questions carried to Spec

- D3 mechanics: does the sugar overlay live as `Arc<Vec<PolicyRecord>>` on
  `AuthzEngine` set at construction, or threaded through each reload call? (Spec
  picks the exact field/signature.)
- G1: one lint call at startup on the merged set replacing the current YAML-only
  call, or an additional merged-set lint alongside it? (Avoid double-WARNing YAML
  routes.)
- G3: extend the existing `/policies` page with a second "Tool Scopes" tab, or a
  dedicated route? (UI IA decision for Spec/Plan.)
