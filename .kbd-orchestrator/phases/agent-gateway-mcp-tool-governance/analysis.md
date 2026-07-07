# Analysis — agent-gateway-mcp-tool-governance

_Analyzed: 2026-07-06 · Mode: stack-specified (Rust / cedar-policy 4 / axum / Ory)_
_Research: firecrawl web (MCP authz spec, Cerbos MCP-authz architecture, Ory
Hydra custom-claims) + local source inspection. Within budget._

## Standing constraint (load-bearing)

Support any IdM with a JWKS pathway; **Ory Kratos/Hydra is the standard**;
**federate, never become an IdP.** This governs the delegate-classification call.

## External evidence (what the industry does)

- **MCP is an OAuth 2.1 authorization surface** (modelcontextprotocol.io/…/security/
  authorization). Per-user/agent access control, **audit of who did which action**,
  and **per-user rate-limiting / usage tracking** are named first-class concerns —
  exactly flint-gate's remit.
- **Cerbos's MCP-authz reference architecture** (cerbos.dev/blog/mcp-authorization)
  independently describes flint-gate's *existing* design: an **externalized policy
  engine** queried **per tool-call** (`checkResource`), **`list_tools` filtered by
  the policy decision** ("tools that come back allow are enabled; the rest
  disabled"), **decision logs** for every check, and the **"attenuated set of
  permissions derived from the original user who called the agent"** — i.e. the
  delegation case. flint-gate already does all of this with embedded **Cedar**
  (`authorize_tool_call`, `ACTION_CALL_TOOL`, `Route::"<tool>"`, authz audit,
  shadow/enforce). **Verdict: the architecture is validated; do NOT adopt an
  external PDP (Cerbos/OPA) — embedded Cedar is the equivalent, in-process, no
  network hop, already built.** The industry gap the sources *don't* solve —
  **per-agent token/cost budgets** — is precisely flint-gate's G2 differentiator.
- **Ory Hydra supports custom access-token claims** via the **token/consent hook**
  (ory/hydra#2552; and Hydra's `client_credentials` grant hook). So the
  "Hydra-side claim mapper" option for delegate-classification is real and
  idiomatic — Hydra can stamp an `agent`-marking claim at mint time.

## Build-vs-adopt calls

Every goal is **build-with-existing** (cedar-policy 4, the Redis budget counters,
the `metrics` surface, `BackendUnavailablePosture` — all already in-tree). **No
new dependency is warranted.** The one design decision requiring evidence — the
delegate-classification mechanism — is resolved below.

### G1 — Delegate-classification + tool-scoping · **BUILD (classify) — no new dep**

The tool-authz engine + `list_tools` filtering + audit already exist and match the
Cerbos reference. The only real gap is that a **Hydra-delegate token isn't
classified `Agent`**, so a delegated agent escapes agent tool-policy + budget.

**Open Q1 — delegate-classification shape — DECIDED: gateway-side classification
from the `act` claim, NOT a Hydra claim mapper (default).**
- A **Hydra-side claim mapper** (stamp `flint_kind=agent` in Hydra) works and is
  idiomatic (ory/hydra#2552), but it (a) pushes flint-gate's identity model into
  every operator's Hydra config (fragile, per-deployment), and (b) only covers the
  Hydra-delegate path, not other JWKS IdMs — violating "federate *any* JWKS IdM."
- **Gateway-side classification** keeps flint-gate a pure verifier: a token
  carrying an RFC 8693 **`act` claim** (which the gateway-local exchange already
  stamps, and which Hydra's 8693 exchange also emits) is classified `Agent` at
  *verification* time — no token rewriting, works for any IdM. This honors
  "federate, never an IdP" and is the smaller change. Document the Hydra claim
  mapper as an optional operator enhancement, not the gateway's mechanism.
  *(RECOMMEND — confirm the exact `act`/`flint_kind` precedence in Spec.)*

### G2 — Agent budget scope + fail-closed posture · **BUILD — no new dep**

Add `BudgetScope::Agent` to the existing `BudgetScope` enum (User|Team today) +
key derivation, tied to the G1 classification so delegated agents are budgeted.

**Open Q2 — agent-budget outage posture — DECIDED: reuse `BackendUnavailablePosture`;
default Agent → Deny (fail-closed), User/Team keep the current degrade.** The
budget resolver is deliberately fail-OPEN today ("a transient blip never
hard-blocks live traffic"). For a **governance** budget that's wrong — an
over-budget agent must not slip through on a Redis/DB error. Reuse the enum built
last phase (`config/types.rs:157`); a per-scope posture (Agent=deny, User=degrade)
preserves human-traffic availability while making agent spend a hard control.
*(RECOMMEND — confirm in Spec.)*

### G3 — Tool-authz + budget metrics · **BUILD (extend `metrics`) — no new dep**

`record_authz_decision` writes only the DB audit trail; add Prometheus counters on
the existing admin `/metrics`: `flint_tool_authz_total{decision}` +
budget-consumption. Reuse the `record_delegate` pattern.

**Open Q3 — tool-label cardinality — DECIDED: do NOT label by raw tool name.**
Tool names are runtime/attacker-influenced (an unknown tool would explode
cardinality — the same trap the delegate metric avoided with `&'static str`).
Label by `decision` (allow/deny — bounded) and optionally an `enforce|shadow`
mode; keep the tool name in the **DB audit** (already there), not the metric.
*(DECIDED.)*

### G4 — Operability edge · **BUILD — no new dep, optional**

Strict cross-replica rate-limit mode (token-endpoint `deny`) + a symmetric
local-mint exchange metric. Small; only if 1–3 land with budget.

### Open Q4 — ergonomics scope · **RECOMMEND: out of scope this phase.**
The admin UI already has a Policies tab + Cedar write-time validation. A
higher-level "agent may call tools [...]" affordance is polish; defer it so the
phase stays on the governance core (classification + budget + metrics). Revisit if
G1–G3 land early. *(RECOMMEND.)*

## Net effect on scope

**Zero new dependencies.** Every goal builds on cedar-policy 4, the shared Redis
budget counters, the `metrics` surface, and `BackendUnavailablePosture` — all
in-tree. Confidence **high** (local-source-confirmed + industry-validated
architecture). The phase is **classification + Agent-budget-scope + fail-closed
posture + metrics**, not new infrastructure.

## Open questions — resolved here

1. Delegate-classification → **gateway-side `act`-based classification** (not a
   Hydra claim mapper); honors federate-never-an-IdP + covers any JWKS IdM. *(confirm precedence in Spec)*
2. Agent-budget posture → **reuse `BackendUnavailablePosture`; Agent=Deny default, User=degrade.** *(confirm in Spec)*
3. Tool metric labels → **`decision`-only, no raw tool name** (cardinality). *(DECIDED)*
4. Ergonomics → **out of scope this phase.** *(RECOMMEND)*

No contested stack, no new library adoption — no elicitation required.
