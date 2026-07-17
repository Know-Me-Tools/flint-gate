# add-tool-authz-metrics

**Phase:** agent-gateway-mcp-tool-governance · **Goal:** G3 (build-003) + G4 stretch (build-004)
**Scope:** `crates/flint-gate-core/src/metrics.rs` (new counters),
`crates/flint-gate-core/src/authz/tool_authz.rs` + `middleware/pipeline.rs`
(instrument), docs.
**Depends on:** changes 1–2 (so agent-scoped decisions/budgets are meaningful to
observe).

## Why

Per-tool authz decisions + budget consumption are recorded only in the **DB audit
trail** (`record_authz_decision`) — there is no Prometheus metric. The `/metrics`
surface (built last phase) exposes only `flint_delegate_*`. Operators can't see
agent tool-call behavior at a glance.

## What

Extend the `metrics` module (reuse the `record_delegate` pattern) with counters on
the existing **admin `/metrics`** (never the proxy port):

1. `flint_tool_authz_total{decision}` — one increment per per-tool-call authz
   decision, labelled `allow`/`deny` (+ optionally `enforce`/`shadow` mode).
2. A **budget-consumption** counter/gauge (e.g. `flint_agent_budget_denied_total`)
   so over-budget denials are visible.
3. **(G4 stretch) — DEFERRED.** A symmetric `flint_local_exchange_total{result}`
   for the gateway-local mint path was scoped as "include only if it fits." The
   local-mint path is `?`-propagation (verify → downscope → mint), so per-outcome
   metering would mean restructuring a fail-closed path late in the phase for a
   low-value symmetric counter. Deferred as documented phase debt rather than
   risk-editing the exchange path; the delegate path remains metered.

**Decided (analysis):** label by `decision` (bounded) — **NOT** raw tool name.
Tool names are runtime/operator-controlled; a raw-name label explodes Prometheus
cardinality (the same trap the delegate metric avoided with `&'static str`). The
tool name stays in the DB audit, which already records it.

## Non-goals

- OTLP export (Prometheus text only).
- Per-tool-name labels (cardinality — decision-only).

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; a test asserting the counter renders after an allow + a deny and is
admin-port-only; label set is bounded/static.
