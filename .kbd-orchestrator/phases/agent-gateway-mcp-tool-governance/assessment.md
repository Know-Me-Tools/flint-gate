# Assessment — agent-gateway-mcp-tool-governance

_Assessed: 2026-07-06 · Backend: openspec · against `goals.md` (4 goals)_
_Method: static inspection at commit `0f4aacb`._

## Headline

The **per-tool-call authorization core already exists and is wired** — `authorize_tool_call`
+ `ToolAuthzContext` run in the streaming pipeline, fail-closed, with a Cedar
`call_tool` action, per-tool `Route::"<tool_name>"` resource, audit trail, and
shadow/enforce mode. So G1 is **mostly built**; its real gaps are the
**delegate-classification** fix and Cedar **ergonomics**. The sharper, genuinely-
new work is in **G2**: budgets have **no `Agent` scope** and resolve **fail-OPEN**.
G3 (authz-decision metrics) is a real observability gap. This mirrors the prior
phase's "already-built-but-unwired" lesson — the assessment reframes the goals
accordingly.

## Goal-by-goal gap analysis

### G1 — Per-tool-call authz + Cedar ergonomics · **MOSTLY BUILT — gaps are delegate-classification + ergonomics**

**Already present (evidence):**
- `crates/flint-gate-core/src/authz/tool_authz.rs` — `authorize_tool_call(...)` +
  `ToolAuthzContext::authorize(tool_name, arguments)`; **collapses every failure
  to `AuthzDecision::Deny`** (fail-closed). Cedar model: `Action::"call_tool"`,
  resource `Route::"<tool_name>"`, context `{tool_name, arguments, route_id}`.
  `list_tools` visibility filtering reuses the same action — one policy governs
  both. Per-tool scoping per principal is therefore **already policy-expressible**.
- Wired into `middleware/pipeline.rs` (per-tool-call authz in the stream) with
  `record_authz_decision` audit + enforce/shadow mode (`pipeline.rs:349-454`).
- Cedar engine + schema + write-time validation (`authz/{engine,bundle,validator}.rs`).

**The gap (specific):**
1. **Delegate-classification (carried debt):** a Hydra-delegate-minted token
   carries Hydra's claims, not `flint_kind=agent`, so a delegated agent may not
   classify as `Agent::` and thus escapes agent-scoped tool policy + budget. Fix
   shape is an open question (Hydra claim mapper vs. gateway policy).
2. **Ergonomics:** authoring/validating tool allow-deny policies is raw Cedar; no
   higher-level "agent X may call tools [...]" affordance. Assess whether this is
   in-scope polish or a genuine usability gap (the admin UI has a Policies tab).

**Estimated effort:** S–M (the engine + enforcement exist; this is classification
+ ergonomics, not a new authz path).

### G2 — Runtime cross-replica agent budgets · **REAL GAP — no Agent scope + fail-OPEN**

**Already present:** windowed budgets via `RedisRateLimiter::incr_budget`/`get_budget`
(shared, cross-replica), wired into the pipeline: `resolve_budget_usage` →
`budget_exceeded(used, limit)` → **block the request** (`pipeline.rs:349-358`),
with a Postgres fallback.

**The gaps (two, both real):**
1. **No `Agent` budget scope.** `BudgetScope` is **`User | Team` only**
   (`config/types.rs:862`). Agent spend cannot be independently budgeted — an
   agent is accounted as its `User`. Add `BudgetScope::Agent` (+ key derivation)
   so agent budgets are first-class, and tie it to the G1 classification so
   delegated agents are covered (closes the delegate-budget-bypass debt).
2. **Budget resolution is fail-OPEN.** `resolve_budget_usage` returns `0`
   (→ allow) on any Redis/DB error, by explicit design ("a transient blip never
   hard-blocks live traffic", `pipeline.rs:1150-1200`). For a **governance**
   budget this contradicts the phase's fail-closed discipline. Add a posture
   (mirroring `oauth.rate_limit.on_backend_unavailable` from last phase): agent
   budgets should default **deny** on a backend outage, while user budgets may keep
   the availability-first degrade.

**Estimated effort:** M (new scope + key + a fail-closed posture toggle + tests).

### G3 — MCP tool-call observability · **REAL GAP**

`record_authz_decision` (`pipeline.rs:1094`) writes only to the **DB audit
trail** — there is **no Prometheus metric** for per-tool authz decisions or
budget consumption. The `metrics` surface (built last phase) currently exposes
**only** `flint_delegate_*`. Add `flint_tool_authz_total{decision,tool}` (bounded
labels — tool name is operator-controlled, cardinality-manageable) + a budget-
consumption gauge/counter, on the existing admin `/metrics`.

**Estimated effort:** S–M (the metrics module + admin endpoint exist; this is
new call sites + label-cardinality care).

### G4 — (Optional) deferred operability edge · **REAL, LOW**

Strict cross-replica rate-limit mode (token-endpoint `deny` posture) + a
symmetric metric for the local-mint exchange path. Small; do only with spare
budget after 1–3.

## Cross-cutting observations

- **Reuse-first is strong again** — the tool-authz engine, budget primitives,
  metrics surface, and audit trail all exist. This phase **wires + extends +
  classifies**, it does not build an authz engine. Guard against re-implementing
  `authorize_tool_call` or the budget counters.
- **Two fail-OPEN seams to close** — the budget DB-read (`pipeline.rs:1198`) and
  the whole `resolve_budget_usage` return-0-on-error. Each new agent-budget path
  needs a `degrades_to_deny` test; this is the phase's central security theme.
- **`&'static str` metric-label discipline** (from last phase) must extend to the
  new authz/budget metrics — but tool name is a runtime `String`, so cardinality
  needs an explicit bound (e.g. only known/configured tool names, or a hashed/
  bucketed label) rather than raw tool strings.
- **Delegate-classification is the linchpin** — it gates both G1 (agent tool
  policy) and G2 (agent budget) for delegated tokens. Resolve its shape in Analyze
  before spec'ing G1/G2.

## Open questions for Analyze/Spec

1. **Delegate-classification fix:** Hydra-side claim mapper (gateway stays a pure
   verifier — honors "federate, never an IdP") vs. an explicit gateway policy path
   for delegate-mode tokens. Which governs delegated agents without the gateway
   minting identity?
2. **Agent-budget fail-closed posture:** a per-scope toggle (Agent → deny on
   outage, User → degrade) vs. a global budget posture. Reuse the
   `BackendUnavailablePosture` enum from last phase?
3. **Tool-authz metric label cardinality:** raw tool name vs. configured-tool
   allowlist vs. bucketed — bound it so a hostile/unknown tool name can't explode
   cardinality.
4. **Ergonomics scope:** is a higher-level agent-tool policy affordance (UI or
   config sugar) in-scope this phase, or is raw Cedar + validation sufficient?

## Recommended change ordering (for Plan)

1. **G1 delegate-classification + tool-scoping confirmation** (BUILD FIRST — it
   gates agent policy AND budget for delegated tokens; the authz engine is done,
   so this is classification + any ergonomics).
2. **G2 Agent budget scope + fail-closed posture** (the genuinely-new governance
   control; depends on G1 classification).
3. **G3 tool-authz + budget metrics** (observe 1–2; the metrics surface exists).
4. **G4 operability edge** (optional polish).

Success criteria in `goals.md` remain valid; G1's is now largely a
classification/ergonomics criterion (the enforcement engine already exists), and
G2's central win is **Agent scope + fail-closed**, not the counter itself.
