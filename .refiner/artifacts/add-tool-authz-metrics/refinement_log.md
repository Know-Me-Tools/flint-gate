# Refinement Log — add-tool-authz-metrics

_Change 3/3 (final) of `agent-gateway-mcp-tool-governance` (Goal G3 + G4 stretch)._
_QA gate: artifact-refiner constraint validation + targeted security self-check._
_(Full agent security review skipped per kbd-execute policy — additive metrics,
no auth-logic seam — after verifying the two security-relevant invariants.)_

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Metric labels are static literals; tool name never a label (leak-tested); no secret. |
| Never expose the admin server (4457) to public internet | PASS | `/metrics` is on the ADMIN router only (`admin/mod.rs:97`), never proxy_app. |
| Never break existing unit tests without updating them | PASS | 420 core tests green; tool_authz + pipeline tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | No config change; metric emitters only. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 420 core tests, 0 failed (8 ignored).
- New-code coverage: render exposes tool-authz + budget-denied metrics;
  end-to-end `authorize()` emits allow/deny/deny_revoked AND the render provably
  contains NO tool name (leak/cardinality guard).

## Targeted security self-check (additive metrics)

- **Label cardinality/injection — SAFE.** Every `record_tool_authz` call site
  passes a `&'static str` literal (`"allow"`/`"deny"`/`"deny_revoked"`); the fn
  signature `record_tool_authz(decision: &'static str)` **structurally forbids** a
  runtime tool name / attacker value as a label. `flint_agent_budget_denied_total`
  is a bare counter (no label). Bounded label set, no unbounded cardinality.
- **Tool-name leak — TESTED.** `authorize_emits_tool_authz_metric_for_allow_deny_and_revoked`
  asserts the render contains none of the tool names passed to `authorize()`.
- **/metrics admin-only — UNCHANGED.** Confirmed `/metrics` appears only in
  `admin/mod.rs`, never the proxy app (all new metrics surface through it).
- **No secret logged** — metric emitters take no request data.

## Delivered

- `flint_tool_authz_total{decision}` at the single per-tool-call authz funnel
  (`ToolAuthzContext::authorize`) — allow / deny / deny_revoked.
- `flint_agent_budget_denied_total` at both agent-budget block points (fail-closed
  outage deny + over-limit block when agent scope/principal).
- **G4 stretch (`flint_local_exchange_total`) DEFERRED** — the local-mint path is
  `?`-propagation; per-outcome metering would mean restructuring a fail-closed
  exchange path late in the phase for a low-value symmetric counter. Documented as
  phase debt (proposal.md); the delegate path stays metered.
- Docs: README observability section (tool-authz + budget metrics, decision-only
  label rationale, admin-port-only).

## Outcome

**PASS** — all constraints satisfied; the two security-relevant invariants
(static-label cardinality, admin-only) verified + tested; G4 stretch deferred with
a documented rationale. Proceed to archive.
