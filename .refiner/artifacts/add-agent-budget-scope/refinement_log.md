# Refinement Log — add-agent-budget-scope

_Change 2/3 of `agent-gateway-mcp-tool-governance` (Goal G2 · build-002)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets; new WARN logs an identity id (pre-existing class), never a token. |
| Never expose the admin server (4457) to public internet | PASS | Unrelated (budget accounting). |
| Never break existing unit tests without updating them | PASS | 418 core tests green; budget/collect tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | Adds a `BudgetScope::Agent` variant only. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 418 core tests, 0 failed (8 ignored).
- `--no-default-features` (redis-l2 OFF) build — compiles; fail-closed path holds.
- New-code coverage: BudgetScope::Agent serde + distinct-key-namespace,
  agent-scope collection keys the agent id, `budget_outage_denies` Agent-only,
  `Unavailable != Known(0)`, and `outage_must_deny` defense-in-depth (agent
  scope OR agent principal).

## What changed (the fail-OPEN → fail-CLOSED seam)

`resolve_budget_usage` returned `u64` with `0`-on-backend-error = silent ALLOW.
Now returns `BudgetUsage::{Known(u64), Unavailable}`; the check site applies
`outage_must_deny(scope, identity)` — **Agent scope OR actual Agent principal →
DENY** on outage; User/Team humans → degrade+WARN. `BudgetScope::Agent` added
(distinct `flint:budget:agent:…` key).

## Separated security review (security-reviewer agent)

Verdict: **no CRITICAL/HIGH** — all three design intents hold; key isolation +
read/write symmetry correct; the `pg_interval None → Known(0)` branch is dead for
the windowed path (Lifetime returns earlier) so NOT an agent fail-open; the
redis-l2-off build is fail-closed for agents; over-budget still blocks.

### MEDIUM — scope-authority gap — FIXED (defense in depth)

The review flagged that fail-closed was bound to operator-declared `config.scope`,
so a delegated agent hitting a `scope: user` (or default) budget silently degraded
on outage. **Remediation (this change, pre-archive):** `outage_must_deny` now
denies when EITHER the budget is agent-scoped OR the request's actual principal is
an `Agent` (`derived_kind`), so a mis-scoped route can't let an agent escape
fail-closed. Covered by `outage_must_deny_covers_agent_scope_and_agent_principal`.

### LOW — accepted / documented

- **Lifetime + Agent no fail-closed** — a `lifetime` agent budget is ledger-only
  (best-effort read); fail-closed is scoped to windowed counters by design.
  **Documented** (README + `outage_must_deny` doc: agent fail-closed needs a fixed
  window).
- **Error Display in WARN** — `%e` on Redis/DB errors; sqlx/redis redact
  connection strings by default; no secret introduced. Noted for future.

## Outcome

**PASS** — the fail-OPEN budget seam is closed for agents; the review's MEDIUM
scope-authority gap was remediated with a principal-kind defense-in-depth check
before archive; LOWs documented. Proceed to archive.
