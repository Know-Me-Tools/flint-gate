# Assessment — agent-gateway-budget-and-policy-operability

_Assessed: 2026-07-07 · Backend: openspec · against `goals.md` (4 goals)_
_Method: static inspection at commit `70fdc36`._

## Headline

Unlike the prior three phases, **little of this phase is already built** — these
are genuine operability gaps, not unwired code. But each has a **strong existing
template**: G1 mirrors the established startup-posture pattern (`admin_auth_posture`
/ `oauth_exposure_posture` → `bail!`), G2's metric reuses `record_delegate`, and
G3 compiles against the existing Cedar write-time validator. "Agent-reachable
route" detection is cleanly derivable from the auth-provider type.

## Goal-by-goal gap analysis

### G1 — Config-validation lints for agent governance · **REAL GAP — clear template**

**Already present (template):**
- A startup-validation pattern on `GateConfig`/`ServerConfig`:
  `admin_auth_posture()`, `oauth_exposure_posture()`, `validate_upstream_url_scheme()`
  (`config/types.rs`), each consumed in `main.rs` via `anyhow::bail!` /
  `warn!` (`main.rs:356, 387, 441, 711`). This is exactly the shape a route-budget/
  policy lint should take.
- `RouteConfig` exposes `auth: Option<String>` + `hooks` (so a route's resolved
  auth provider + its `MaxTokenBudget` / `Authorize` hooks are inspectable).
- **Agent-reachable detection is derivable:** `AuthProviderConfig` variants are
  `Kratos | Jwt | ApiKey | Anonymous | Mcp` (`config/types.rs:501`). A route is
  **agent-reachable** iff its resolved provider is **`Jwt` or `Mcp`** (JWKS-backed
  → can carry an RFC 8693 `act`/agent token). Kratos=human, ApiKey=Service,
  Anonymous=none.

**The gap:** no lint inspects routes for the agent-governance under-application the
prior phase flagged (debt #3):
1. an **agent-reachable** route whose `MaxTokenBudget` hook is left at a non-agent
   `scope` (agent spend silently accounted in the user keyspace);
2. an agent-reachable route with **no per-tool `Authorize` hook** at all (ungoverned
   tool calls).
Add `GateConfig::agent_governance_lint() -> Vec<Finding>` + a severity model
(WARN default, opt-in strict → `bail!`), consumed at startup.

**Estimated effort:** M (route-walk + provider resolution + severity posture +
main.rs wiring; the posture precedent makes it mechanical).

### G2 — Symmetric exchange metrics + strict cross-replica rate-limit mode · **REAL GAP (2 parts)**

- **Local-mint metric:** confirmed **unmetered** — only `record_delegate*` exists
  in `token_exchange.rs` (the delegate branch). The gateway-local mint path
  (`verify → downscope → mint`) is `?`-propagation with no per-outcome counter.
  Add `flint_local_exchange_total{result}`; the `?`-chain needs light
  restructuring into outcome arms (the deferred debt #1).
- **Strict cross-replica rate-limit mode:** `oauth.rate_limit.on_backend_unavailable`
  (`Deny|Degrade`) already governs the token endpoint's behavior when the shared
  limiter **errors** mid-request. But there is **no knob to refuse/deny when a
  shared cross-replica limiter is not configured at all** (redis-l2 off or no
  `cache.l2`) — today `build_oauth_routes` silently falls back to the per-replica
  in-process governor (`main.rs:847` no-redis-l2 build). The "strict mode" is a new
  startup/config posture: refuse-start (or deny) if strict cross-replica limiting
  is required but no shared backend is configured. Distinct from the existing
  on-error posture.

**Estimated effort:** M (metric + `?`-restructure; strict-mode posture + startup
check — ties to G1's posture pattern).

### G3 — Cedar ergonomics for agent tool-scoping · **REAL GAP**

Only **raw Cedar** exists (`authz/{bundle,validator}.rs` — policies are hand-written
`permit(principal == Agent::"x", action == Action::"call_tool", …)`). The
**write-time validator** (`authz/validator.rs`) is the compile target. Add a config
sugar — e.g. a per-agent `tools: { allow: [...], deny: [...] }` block — that
**compiles to Cedar** and is validated by the existing validator before load. The
admin Policies tab is the UI surface (optional this phase).

**Estimated effort:** M–L (sugar schema + Cedar codegen + validation wiring; UI is
the cost driver if included).

### G4 — Fail-closed lifetime agent budgets · **REAL, LOW**

`resolve_budget_usage` returns `Known(lifetime_usage_from_lookups(...))` for the
Lifetime window — best-effort (allow-on-error), never `Unavailable`. So an
`scope: agent` + `window: lifetime` budget is NOT fail-closed. Two options (decide
in Analyze): make the lifetime lookup return `Unavailable` on error for agents, OR
**refuse the combination at config-validation time** (ties to G1's lint). The
refuse-at-config option is simpler and consistent with "fail-closed agent budgets
require a fixed window" (already documented).

**Estimated effort:** S.

## Cross-cutting observations

- **Reuse-first via the posture pattern** — G1, G2-strict, and G4-refuse all fit
  the `*_posture() → bail!` template already in the tree (admin/oauth). Build one
  `agent_governance_lint` + a severity enum, not three ad-hoc checks.
- **Fail-safe defaults** — the lint should default to **WARN** (don't break
  existing loose configs) with an opt-in strict/refuse posture, mirroring the
  admin-auth precedent (loopback allowed by default, refuse-start only when
  exposed). Each new lint/posture needs a fail-safe-default test.
- **G1 and G4 converge** — a config-validation lint is the natural home for both
  the budget-scope warning AND the lifetime+agent refusal. Spec them together.
- **Separated security review** still applies to G1/G2/G4 (they change
  startup/deny behavior), lighter for G3 (policy authoring, validated by the
  existing Cedar validator).

## Open questions for Analyze/Spec

1. **Lint severity default:** WARN-by-default + opt-in `strict`(refuse-start), vs.
   refuse-by-default. (Recommend WARN default per the admin precedent — confirm.)
2. **Agent-reachable detection precision:** provider-type (`Jwt`/`Mcp`) is the
   clean signal, but a route can also inherit `default_auth` from its site. Resolve
   the route→site→provider chain; also decide whether an `Anonymous`/`ApiKey` route
   that could still receive a bearer token counts. (Provider-type is the
   recommended, least-surprising signal.)
3. **G3 sugar shape:** config-block sugar vs. admin-UI builder vs. both; is UI in
   scope this phase or deferred?
4. **G4:** fail-closed lifetime read vs. refuse-at-config. (Recommend refuse-at-
   config — simpler, folds into G1's lint.)

## Recommended change ordering (for Plan)

1. **G1 agent-governance lint + severity posture** (BUILD FIRST — the sharpest
   silent-under-application edge; establishes the posture G4 folds into).
2. **G4 lifetime+agent refusal** (fold into G1's lint — smallest, consistent).
3. **G2 local-mint metric + strict cross-replica mode** (observability + the
   strict posture).
4. **G3 Cedar tool-scope sugar** (ergonomics; largest, least security-sensitive).

Success criteria in `goals.md` remain valid; G1 is the anchor and G4 naturally
merges into it.
