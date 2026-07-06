# add-delegate-observability

**Phase:** agent-gateway-exposure-operability · **Goal:** G2 (cand-001 + build-003)
**Scope:** `crates/flint-gate-core/Cargo.toml` (deps),
`crates/flint-gate-core/src/auth/token_exchange.rs`,
`crates/flint-gate-core/src/auth/oauth.rs`,
`crates/flint-gate/src/main.rs` (admin `/metrics` route + recorder), docs.

## Why

Delegate-mode token exchange is a blind spot: no metrics facility exists at all,
so operators can't see delegate volume, denials, or the fact that Hydra-minted
delegated tokens bypass the gateway's `flint_kind` agent-budget classification.

## What

**Adopt** `metrics` 0.24 (facade) + `metrics-exporter-prometheus` 0.18. Install a
Prometheus recorder at startup; serve `render()` on a **`/metrics` route on the
admin port** (private control-plane surface — NOT the public proxy). Instrument
the delegate paths in `token_exchange.rs`:
- `flint_delegate_total{result="success"|"deny", reason}` counter,
- a delegate-latency histogram.

**Decision encoded (build-003):** do **NOT** re-stamp Hydra-minted delegated
tokens with `flint_kind=agent` — rewriting another authority's token is IdP
behavior and violates the federate-never-an-IdP constraint. Instead, document that
delegate-mode tokens carry Hydra's claims (outside gateway agent-budget
classification) and let the new `flint_delegate_total` metric make that bypass
volume visible.

## Non-goals

- OTLP export (Prometheus text only this change).
- Full HTTP-request metrics for every route (delegate-semantic counters only).
- Re-stamping delegated tokens (explicitly rejected).

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; a test asserting `/metrics` renders the delegate counters and that
the endpoint is on the admin port, not the proxy port.
