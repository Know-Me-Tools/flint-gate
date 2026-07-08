# add-local-exchange-metric-strict-ratelimit

**Phase:** agent-gateway-budget-and-policy-operability · **Goal:** G2 (build-002 + build-003)
**Scope:** `crates/flint-gate-core/src/metrics.rs` (metric),
`crates/flint-gate-core/src/auth/token_exchange.rs` (local-mint instrumentation),
`crates/flint-gate-core/src/config/types.rs` (strict-mode flag + posture),
`crates/flint-gate/src/main.rs` (startup enforcement), docs.

## Why

Two carried gaps from prior phases:
1. The **gateway-local mint** exchange path is unmetered — only the Hydra-delegate
   branch emits `flint_delegate_*`, so operators can't see local-exchange volume/
   outcomes (debt #1).
2. There is **no strict cross-replica rate-limit mode**:
   `oauth.rate_limit.on_backend_unavailable` only governs a mid-request Redis
   *error*; when no shared limiter is configured at all (redis-l2 off / no
   `cache.l2`), the OAuth surface silently falls back to the per-replica
   in-process governor (carried debt #4).

## What

1. **Local-mint metric** — add `flint_local_exchange_total{result}` (via the
   `record_delegate` `&'static str`-label pattern). Restructure the local
   `exchange()` branch (`verify → downscope → mint`, currently `?`-propagation)
   into outcome arms: `success` / `deny_verify` / `deny_downscope` / `mint_failed`.
   Both exchange modes are then observable.
2. **Strict cross-replica mode** — add `oauth.rate_limit.require_shared_backend:
   bool` (off by default). When true, **refuse to start** if the OAuth surface is
   exposed on a non-loopback bind without a shared Redis limiter configured (folds
   into `oauth_exposure_posture` / the exposure startup check). Turns "I need
   cross-replica-accurate limits" into an enforced invariant.

## Non-goals

- Changing the local-exchange fail-closed behavior (metric only; outcomes unchanged).
- Per-request rate-limit backend changes (this is a startup posture).

## Fail-safe requirement

The `?`-restructure must preserve every existing fail-closed outcome (verify/
downscope/mint failures still deny — guarded by existing tests). `require_shared_
backend: true` + exposed-non-loopback + no shared limiter → refuse start (tested).

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; local-exchange outcome-metric tests + strict-mode refuse-start test;
existing token-exchange tests unregressed.
