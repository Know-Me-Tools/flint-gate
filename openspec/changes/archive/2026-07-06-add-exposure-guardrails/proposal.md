# add-exposure-guardrails

**Phase:** agent-gateway-exposure-operability · **Goal:** G3 (build-002)
**Scope:** `crates/flint-gate-core/src/config/types.rs`,
`crates/flint-gate-core/src/auth/token_exchange.rs`,
`crates/flint-gate/src/main.rs`, docs.

## Why

Last phase documented three exposure caveats as accepted risk; this change turns
them into enforced invariants so `/oauth/*` cannot be misconfigured into an
unsafe exposure. No new dependency — mirror the existing `admin_auth_posture()`
fail-safe pattern.

## What

1. **https-only upstream URLs** — validate `hydra_token_url` / `hydra_admin_url`
   scheme at startup; reject `http://` unless a new, off-by-default
   `allow_insecure_upstream` config field is explicitly set (loud WARN when on).
2. **Hydra-response body cap** — replace the unbounded `resp.json::<Value>()` in
   the delegate/introspection paths with a size-limited read (default 64 KiB)
   then parse; over-cap → fail-closed (`MintFailed`/deny).
3. **`/oauth/*` exposure posture** — add `oauth_exposure_posture()` mirroring
   `admin_auth_posture()`: `RefuseStart` when `/oauth/*` would bind non-loopback
   without **both** `introspect_auth` and rate-limiting configured; `AllowLoopback`
   on a loopback bind; `Enforce` otherwise.

## Non-goals

- mTLS to upstreams (separate concern).
- Reworking the admin posture (only mirroring its shape).

## Fail-closed requirement

`http://` Hydra URL without override → refuse start; over-cap Hydra body → deny;
non-loopback `/oauth/*` without introspect_auth+rate-limit → refuse start. Each
gets a test.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage on new code.
