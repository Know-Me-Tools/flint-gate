# Assessment — agent-governance-completeness-and-policy-authoring

_Assessed 2026-07-07 against `goals.md`. Codebase inspected via targeted
exploration of the route pipeline, the Cedar engine load/reload paths, and the
admin web UI + policy API. Workspace is green at entry (456 core tests; prior
phase archived, uncommitted)._

## Summary

All three goals are **structural follow-throughs on code this project already
has** — the hard primitives (the reusable lint helper, the sugar→Cedar compiler,
the merge-capable bundle, the admin policy CRUD + web Policies page) exist and
are tested. The work is **wiring**, not invention:

- **G1** — `agent_governance_lint_routes(&[RouteConfig])` already lints an
  arbitrary route slice; nothing calls it with the DB-merged set. The merged set
  exists only *transiently* and is thrown away.
- **G2** — `CedarBundle::from_records` already merges any `&[PolicyRecord]` into
  one PolicySet; the sugar already compiles to `Vec<PolicyRecord>`. The gap is
  that the sugar records are not carried into the DB-backed engine's build/reload
  paths, so a `refuse-start` guard stands in for the merge.
- **G3** — the allow/deny→Cedar compiler exists server-side but is YAML-only:
  no admin route, no client fn, no UI. The Policies page accepts only raw Cedar;
  AgentIdentities + Routes are the two existing structured-form UI patterns to
  follow.

The sharpest design constraints are in the **reload error model** (G1: you cannot
`bail!` a live process) and **merge precedence + reload persistence** (G2: every
reload path must re-inject the sugar or it silently drops).

---

## G1 — Lint + govern DB-sourced routes  ·  Verdict: NOT MET (gap confirmed)

**What exists:**
- `agent_governance_lint()` (`config/types.rs:151`) walks `self.routes` (YAML)
  only — its own doc comment flags the DB gap and points at the reusable helper.
- `agent_governance_lint_routes(&self, routes: &[RouteConfig])`
  (`config/types.rs:157`) — pure, dedups, filters `r.enabled`; **already accepts
  any route slice.** This is the reuse hook.
- Called once at startup: `main.rs:170` (WARN each; `bail!` under
  `strict_agent_governance` at `main.rs:174-185`). Startup `bail!` is safe
  (pre-serve).
- DB routes: `Database::load_routes()` (`db/mod.rs:399`) →
  `Router::from_config_and_db_routes(config, db_routes)` (`proxy/router.rs:95`)
  honored under `database.override_yaml` at `main.rs:257` (startup) and
  `main.rs:810` (config-file reload). **The merged `Vec<RouteConfig>` is built at
  `router.rs:128` and immediately consumed by `from_config` — never surfaced.**
- Hot-reload: `start_cache_invalidation_listener` (`cache/mod.rs:326`) →
  `rebuild_router_from_db` (`cache/mod.rs:398`) on a `"routes"` NOTIFY re-loads DB
  routes and rebuilds the whole `Router` via the same merge (`cache/mod.rs:406`).
  This site holds **both** `config` and freshly-loaded `db_routes` — the natural
  place to lint the merged set on reload.

**Gap:** the lint only ever sees YAML. A DB-only under-governed agent route (an
agent-reachable `gate_routes` row with a non-agent budget / no authorize hook)
passes `strict_agent_governance` at boot and is never surfaced.

**Hard constraint (reload error model):** the reload path is best-effort and has
**no `bail!`/exit anywhere** — bad DB route → WARN-skip (`router.rs:117`);
`load_routes` failure → router unchanged (`cache/mod.rs:411`). A live process can
only **rebuild-or-retain**; it cannot refuse to keep running. So reload-time
governance can WARN, or reject-a-route-and-retain-last-good, but **cannot
terminate** — unlike the startup `bail!`. This is the crux the Analyze/Spec
stages must resolve (open question in goals.md).

**Effort:** LOW–MEDIUM. Startup: lint the merged set (call the existing helper
with the merged `Vec<RouteConfig>` — needs surfacing it out of
`from_config_and_db_routes`, or recomputing the merge for the lint). Reload:
lint at `rebuild_router_from_db` and WARN (strict-mode reject-and-retain).

---

## G2 — Merge config `agent_tool_policies` into the DB-backed engine  ·  Verdict: NOT MET (refuse-start guard stands in)

**What exists:**
- Sugar compiler: `compile_agent_tool_policies` / `compile_and_validate`
  (`authz/sugar.rs:72,139`) → `Vec<PolicyRecord>`; ids namespaced
  `agent_tool_sugar::<agent>::<index>` (`sugar.rs:95`) — **collision-safe against
  DB rows** already.
- `CedarBundle::from_records[_lenient]` (`bundle.rs:91,116`) merges any
  `&[PolicyRecord]` into one PolicySet (each statement re-id'd `"{id}#{idx}"`,
  `bundle.rs:188`); **the merge primitive already exists.** Schema/entities are
  first-non-null-wins; sugar records carry `None`, so DB schema still wins.
- Engine: `from_database` (`engine.rs:210`) builds lenient from
  `db.load_enabled_policies()` (`db/mod.rs:1022`, `SELECT ... FROM authz_policies
  WHERE enabled`); `from_records` (`engine.rs:194`, strict);
  `reload_from_records_lenient` (`engine.rs:245`, ArcSwap store);
  `reload_from_database` (`engine.rs:229`, DB-only).
- Current wiring (`main.rs`): `4c.` compiles sugar (`main.rs:192`); a
  **refuse-start guard** rejects `db.is_some() && !sugar_policies.is_empty()`
  (`main.rs:306-315`); engine built DB-only in the `Some(d)` arm, sugar-seeded
  only in `None if !sugar_policies.is_empty()` (`main.rs:316-323`).
- **Policies reload drops sugar:** `"policies"` NOTIFY → `reload_from_database`
  (`cache/mod.rs:372`, DB-only); admin CRUD also calls `reload_from_database`
  (`admin/mod.rs:727,748`). Any sugar merged at startup would be **dropped on the
  first reload.**

**Gap:** the sugar is enforced only in the config-only (no-DB) deployment; with a
DB it refuses-start. To enforce alongside DB policies, `sugar_policies` must be
carried into **every** engine build/reload path (startup DB arm + the two
`reload_from_database` call sites), and the refuse-start guard removed.

**Design decisions the Spec must fix (open questions in goals.md):**
- **Reload persistence:** store `sugar_policies` on the `AuthzEngine` (or thread
  it) so `reload_from_database` concatenates DB records + sugar before the swap —
  else every reload drops it. This is the central mechanical change.
- **Precedence:** Cedar `forbid` overrides `permit` regardless of source, so a DB
  `forbid` still beats a sugar `permit` — but define/behaviorally-test
  sugar-permit vs DB-forbid and two-permit cases. PolicyId namespaces already
  don't collide.
- **Single source vs overlay:** keep sugar an in-memory overlay rebuilt from
  config on reload (simplest, config stays source of truth) vs. writing compiled
  sugar into `authz_policies` rows (unifies with the admin UI, but muddies "who
  owns this policy"). Overlay is the lower-risk default.

**Effort:** MEDIUM. No bundle-level change (merge primitive exists); the work is
assembling `DB records ++ sugar` at ~3 call sites + carrying the sugar for reload,
+ removing the guard, + precedence tests.

---

## G3 — Admin-UI policy builder for agent tool-scopes  ·  Verdict: NOT MET (server-side compiler exists; no admin surface)

**What exists:**
- Web stack: React 19 + Vite 6 + TS, React-Router 7, TanStack Query 5, Tailwind
  v4, shadcn-style kit (`web/package.json`). UI calls `/api/*` → admin bind
  `:4457` (`web/vite.config.ts`).
- **Policies page exists but is raw-Cedar-only:** `web/src/pages/Policies.tsx`
  (`/policies`, `App.tsx:78`) — table + `PolicyForm` with a plain Cedar
  `<Textarea>` + Schema/Entities JSON textareas (`Policies.tsx:178-191`). No
  allow/deny builder.
- Policy CRUD API: `/policies` GET/POST, `/policies/{id}` GET/PUT/DELETE
  (`admin/mod.rs:130-139`); `upsert_policy_inner` (`mod.rs:680`) calls
  `validate_policy` (400 on invalid, fail-closed), collects `policy_warnings`,
  persists, then `reload_from_database`. Request `UpsertPolicyRequest`
  (`mod.rs:602`).
- **Reference UI patterns:** `AgentIdentities.tsx` (page + table + modal
  **structured builder form** with a segmented kind toggle + React-Query hooks) —
  closest pattern, and its principals are exactly what a tool-scope names.
  `Routes.tsx` — nested structured-config form (`hooks.pre_request[]` arrays)
  POSTed as one JSON object — the template for authoring allow/deny lists.
- **The compiler is server-side but unexposed:** grepping `admin/` + `web/src/`
  for `agent_tool_policies`/`compile_agent_tool_policies`/`sugar`/`tool_scope` →
  **no matches.** `compile_agent_tool_policies` (`authz/sugar.rs:72`) and
  `AgentToolPolicy` (`config/types.rs:50`) exist only as YAML config.

**Gap:** no admin endpoint and no UI for the allow/deny sugar. A builder needs (a)
an admin API to accept `{agent, allow[], deny[]}` → compile via
`compile_and_validate` → persist/activate, and (b) a Policies-tab (or new tab) UI
form following the AgentIdentities/Routes pattern.

**Dependency:** G3 is cleanest **after G2** — the builder should write into the
same merged/enforced policy set G2 establishes (otherwise the UI authors
something that, like today's config sugar, isn't enforced with a DB). Build order
G1 → G2 → G3 (matches goals.md).

**Effort:** MEDIUM (backend endpoint small — reuse `compile_and_validate` +
existing persist/reload; frontend form is the bulk, but two close templates
exist).

---

## Cross-cutting observations

- **Zero new dependencies expected** — every goal builds on in-tree
  `cedar-policy 4`, the existing lint helper, the sugar compiler, the merge-capable
  bundle, the admin CRUD + React/Query/Tailwind UI kit.
- **Fail-safe discipline carries:** G1 reload must be non-fatal (WARN/retain);
  G2 must stay parse-before-swap fail-closed and preserve deny-wins/injection
  safety; G3 must keep the `validate_policy` 400-gate. Each of the three touches
  authz/policy/reload → **separated security review on each** (per constraints).
- **Blocking constraints unaffected:** no secrets; admin bind stays loopback
  default; no test breakage expected; config priority (CLI>env>YAML) untouched.

## Open questions for Analyze / Spec

1. **G1 reload error model** — WARN-only on reload vs. reject-the-route-and-retain
   under strict mode? (Cannot `bail!` a live process.) How does startup-strict
   (`bail!`) reconcile with reload-strict (can't exit)?
2. **G1 merged-set surfacing** — expose the merged `Vec<RouteConfig>` from
   `from_config_and_db_routes` for the lint, or recompute the merge for linting?
3. **G2 sugar reload persistence** — store `sugar_policies` on `AuthzEngine` and
   concatenate on every `reload_from_database`, vs. write compiled sugar into
   `authz_policies` rows (single source of truth)?
4. **G2 precedence** — define + test sugar-permit vs DB-forbid and two-permit
   conflict outcomes (Cedar forbid-wins is the backstop).
5. **G3 write target** — does the UI builder edit config sugar, write DB policy
   rows, or post `{agent,allow,deny}` to a new endpoint that compiles + stores?
   (Depends on the G2 overlay-vs-DB-row decision.)
