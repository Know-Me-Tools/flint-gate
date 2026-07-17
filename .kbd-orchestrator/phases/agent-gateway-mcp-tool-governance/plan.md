# Plan — agent-gateway-mcp-tool-governance

_Planned: 2026-07-06 · Backend: openspec · Driver: kbd-apply (one task/turn)_
_Inputs: assessment.md, analysis.md (firecrawl-researched), library-candidates.json, 3 OpenSpec specs._

## Ordering rationale (dependency-aware)

The delegate-classification change is the **linchpin**: until a delegated token
classifies as `Agent`, neither agent tool-policy (goal 1) nor agent budget
(goal 2) applies to it. So classification lands first; the agent-budget scope
depends on it; the metrics change observes both. **Zero new dependencies** —
every change reuses in-tree primitives (research confirmed embedded Cedar is the
industry-standard equivalent of an external PDP; no Cerbos/OPA adoption).

## Ordered changes

### 1. `add-agent-delegate-classification` — G1 · **BUILD FIRST (the governance linchpin)**
- **Reuses:** `auth/identity.rs::derived_kind` (existing `flint_kind`/`act`/session
  precedence) — build-001. **No Hydra claim mapper** (decision: gateway-side
  `act`-based classification honors federate-never-an-IdP + covers any JWKS IdM).
- **Delivers:** a verified token with a well-formed RFC 8693 `act` → `Agent`,
  gateway-side, no token rewriting; signed `flint_kind` keeps first precedence.
- **Recommended agent:** security-reviewer (spoofing / privilege-escalation is the
  risk — a bare/forged claim must not promote to Agent).
- **Gate:** `cargo check/clippy -D warnings/test --workspace`; spoof-resistance +
  `act`→Agent (Hydra-delegate + generic JWKS) + malformed-`act`-safe-default tests;
  ≥80% new-code coverage.
- **Tasks:** 5.

### 2. `add-agent-budget-scope` — G2 · **depends on 1**
- **Reuses:** `BudgetScope` enum, `RedisRateLimiter::incr_budget`/`get_budget`,
  `BackendUnavailablePosture` (built last phase), `budget_exceeded` enforcement —
  build-002.
- **Delivers:** `BudgetScope::Agent` + key derivation (tied to change-1
  classification) + a **fail-closed outage posture** (Agent→Deny default,
  User/Team→degrade), closing the fail-OPEN `resolve_budget_usage` seam for agents.
- **Recommended agent:** security-reviewer (fail-open→fail-closed conversion; the
  budget must not be bypassable on a backend blip) + rust-reviewer (key collision).
- **Gate:** `degrades_to_deny` (Agent) + User-still-degrades (no regression) +
  over-budget-Agent-blocked + no-key-collision tests; workspace green; ≥80%.
- **Tasks:** 5.

### 3. `add-tool-authz-metrics` — G3 (+ G4 stretch) · **depends on 1–2**
- **Reuses:** `metrics.rs` (`record_delegate` pattern), the admin `/metrics`
  endpoint — build-003 (+ build-004 stretch: local-mint exchange metric).
- **Delivers:** `flint_tool_authz_total{decision}` (**decision-only** label — tool
  name stays in DB audit, cardinality) + a budget-denial counter; optional
  `flint_local_exchange_total{result}`.
- **Recommended agent:** rust-reviewer (metric wiring); security-reviewer
  light-touch (confirm admin-port-only + no tool-name/credential in labels).
- **Gate:** counter renders on `/metrics` after allow+deny; admin-port-only;
  bounded/static labels; budget-denial counter increments; workspace green; ≥80%.
- **Tasks:** 5.

## Cross-cutting guardrails (apply to every change)

- **Fail-closed discipline** — the phase's central theme is converting the two
  fail-OPEN budget seams to fail-closed for agents; each change carries a
  deny-path/spoof-resistance/`degrades_to_deny` test. Separated security review on
  all three (all touch auth/budget) — the author never grades its own seam.
- **Reuse-first** — do NOT rebuild the authz engine, budget counters, or metrics
  surface (all in-tree; research-validated). Guard against re-implementation.
- **`&'static str` label discipline** — new metrics keep bounded static labels; no
  runtime tool name / credential as a label (cardinality + leak).
- **Standing constraint** — federate any JWKS IdM, Ory reference, never an IdP
  (load-bearing in change 1's gateway-side classification decision).
- **Blocking constraints** (project) — no secrets committed; admin port 4457 not
  public (change 3 `/metrics` = admin-port); no broken tests; config priority
  CLI>env>YAML unchanged.

## Spec confirmations carried into Execute

- Change 1: confirm the exact `flint_kind` vs `act` precedence in `derived_kind`
  before adding the `act`→Agent rule.
- Change 2: the per-scope posture shape (Agent=deny / User=degrade) — a config
  toggle vs. hardcoded per-scope default; confirm at implementation.

## Execution

Backend openspec, `/kbd-apply` one task/turn. Archive each via
`openspec archive <id> --skip-specs --yes`. Per-change QA: artifact-refiner +
security review before archive.

**First change to apply:** `add-agent-delegate-classification`.
