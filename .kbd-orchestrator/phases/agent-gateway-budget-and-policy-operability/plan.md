# Plan — agent-gateway-budget-and-policy-operability

_Planned: 2026-07-07 · Backend: openspec · Driver: kbd-apply (one task/turn)_
_Inputs: assessment.md, analysis.md, library-candidates.json, 3 OpenSpec specs._

## Ordering rationale

The governance lint is the **anchor** — it closes the sharpest silent-under-
application edge (agent spend accounted in the user keyspace) and establishes the
WARN/strict posture that G4 folds into. Changes are otherwise independent (distinct
files/config keys), but 1 lands first for impact; 3 is fully independent and least
security-sensitive. **Zero new dependencies** — the whole phase reuses in-tree
primitives (the posture pattern, the pipeline's route→provider resolution, the
`record_delegate` metric pattern, the Cedar write-time validator).

## Ordered changes

### 1. `add-agent-governance-lint` — G1 + G4 · **BUILD FIRST (the anchor)**
- **Reuses:** the `admin_auth_posture`/`oauth_exposure_posture` → `bail!` pattern;
  the pipeline's effective-auth resolution (`route.auth` ?? `site.default_auth` →
  provider variant) — build-001. G4 (lifetime+agent refusal) folded in.
- **Delivers:** `GateConfig::agent_governance_lint()` flagging agent-reachable
  (`Jwt`/`Mcp`) routes with a non-agent budget / no `Authorize` hook / `scope:agent`
  +`lifetime`; **WARN default + opt-in `server.strict_agent_governance` → bail!**.
- **Recommended agent:** security-reviewer (a lint that changes startup behavior +
  the agent-reachable classification is the risk — false-negative = silent gap).
- **Gate:** `cargo check/clippy -D warnings/test --workspace`; per-finding +
  strict-vs-warn + clean-config-empty + Kratos/ApiKey/Anonymous-no-finding tests;
  ≥80% new-code coverage.
- **Tasks:** 5.

### 2. `add-local-exchange-metric-strict-ratelimit` — G2 · independent
- **Reuses:** `metrics.rs` `record_delegate` pattern; `oauth_exposure_posture` /
  the exposure startup check — build-002 + build-003.
- **Delivers:** `flint_local_exchange_total{result}` (via a `?`→outcome-arms
  restructure of the local exchange) + `oauth.rate_limit.require_shared_backend`
  refuse-start when exposed non-loopback without a shared limiter.
- **Recommended agent:** rust-reviewer (the `?`-restructure must preserve every
  fail-closed outcome) + security-reviewer (strict-mode refuse-start posture).
- **Gate:** local-exchange outcome-metric tests + existing exchange tests
  unregressed + strict-mode refuse-start test; workspace green; ≥80%.
- **Tasks:** 5.

### 3. `add-agent-tool-scope-sugar` — G3 · **fully independent**
- **Reuses:** `authz/validator.rs` (write-time Cedar validation); the
  `call_tool`/`Route` Cedar model — build-004. Admin-UI **deferred**.
- **Delivers:** `agent_tool_policies:{allow,deny}` config compiling to Cedar
  `permit`/`forbid`, validated before load; deny-wins; glob support.
- **Recommended agent:** rust-reviewer (codegen correctness) + security-reviewer
  light-touch (deny-wins + invalid-sugar-rejected — a bad policy must never load).
- **Gate:** allow / deny-override / glob / reject-invalid tests; workspace green; ≥80%.
- **Tasks:** 5.

## Cross-cutting guardrails (apply to every change)

- **Fail-safe discipline** — the lint defaults to WARN (non-breaking) with a
  fail-safe strict escalation; the strict rate-limit + tool-sugar paths fail
  closed (refuse-start / reject-invalid). Each carries a fail-safe-default /
  fail-closed test. Separated security review on 1 + 2 (startup/deny behavior);
  light-touch on 3 (validated by the Cedar validator).
- **Reuse-first** — one `agent_governance_lint` + severity enum, not ad-hoc
  checks; the sugar is a front-end over the existing Cedar engine, not a second
  engine. Guard against re-implementation.
- **`&'static str` label discipline** — the local-exchange metric keeps bounded
  static labels (like the delegate + tool-authz metrics).
- **Standing constraint** — federate any JWKS IdM, Ory reference, never an IdP
  (load-bearing in the sugar-validates-to-Cedar and the JWKS-provider detection).
- **Blocking constraints** (project) — no secrets; admin port 4457 not public;
  no broken tests; config priority CLI>env>YAML unchanged.

## Spec confirmations carried into Execute

- Change 1: the `GovernanceFinding` shape (route id + reason) + the exact
  WARN-vs-strict-bail! wording.
- Change 3: confirm the sugar's glob semantics reuse the engine's existing
  tool-name matching (don't invent a new matcher).

## Execution

Backend openspec, `/kbd-apply` one task/turn. Archive each via
`openspec archive <id> --skip-specs --yes`. Per-change QA: artifact-refiner +
security review before archive.

**First change to apply:** `add-agent-governance-lint`.
