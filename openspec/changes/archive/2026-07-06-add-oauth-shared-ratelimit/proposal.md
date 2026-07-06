# add-oauth-shared-ratelimit

**Phase:** agent-gateway-exposure-operability · **Goal:** G1 (build-001)
**Scope:** `crates/flint-gate/src/main.rs`,
`crates/flint-gate-core/src/ratelimit/`,
`crates/flint-gate-core/src/config/types.rs`, docs.

## Why

The per-endpoint OAuth governor on `/oauth/token` + `/oauth/introspect` is the
in-process `build_governor_layer` (`main.rs:469`), so rate limits are
per-replica and inaccurate across a horizontally-scaled deployment — the
exposure gate for scaling out. A Redis-backed shared limiter already exists
(`RedisRateLimiter`, Lua-atomic fixed-window) but `incr_request` is
`#[allow(dead_code)]` and unwired.

## What

Route `/oauth/*` rate limiting through the shared `RedisRateLimiter::incr_request`
(keyed by `client_id`, peer-IP fallback for the pre-auth token surface), so the
window is authoritative across replicas. Add a Redis-outage posture: a config
toggle `oauth.rate_limit.on_backend_unavailable` = `deny | degrade`, defaulting
to **deny for `/oauth/introspect`** (the token-scanning oracle must not lose its
limit) and **degrade-to-in-process-governor + WARN for `/oauth/token`** (avoid an
availability cliff on a Redis blip). Do **not** adopt a second rate-limit crate —
`tower_governor` is in-process only and cannot be cross-replica.

## Non-goals

- Replacing the in-process governor on the general proxy surface (it stays as the
  degrade target).
- Sliding-window / token-bucket semantics (fixed-window is sufficient and matches
  the existing limiter).

## Fail-closed requirement

Every new path needs a deny-path test: over-window → `429`; Redis-unavailable
under `deny` posture → deny (introspect default); under `degrade` → in-process
governor still enforces + a WARN is emitted.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage on new code; the `#[ignore]`d live-Redis tests documented for CI.
