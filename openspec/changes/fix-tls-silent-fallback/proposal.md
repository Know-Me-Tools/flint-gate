# fix-tls-silent-fallback

**Phase:** beta-release-readiness / Phase 1 (Blocker B-3)
**Priority:** BLOCKER — must close before any external beta

## Problem

When `tls.enabled: true` but cert or key loading fails, the server logs a
`warn!` and falls back to plain TCP. Operators who configure TLS and see
"proxy server listening" in logs will assume TLS is active. Traffic flows
plaintext. There is no config option to make TLS failures fatal.

Affected code: `crates/flint-gate/src/main.rs` lines ~727–752.

## Solution

Add `tls.fail_open: bool` (default `false`) to `ServerConfig`. When
`fail_open: false` (the new default), a cert loading failure calls
`anyhow::bail!()` — the process exits non-zero. When `fail_open: true`,
the current fallback behavior is preserved but an explicit `WARN` is emitted
that names the risk.

## Files to change

- `crates/flint-gate-core/src/config/types.rs` — add `fail_open` field to `TlsConfig`
- `crates/flint-gate/src/main.rs` — branch on `fail_open` in the TLS setup block
- `config.example.yaml` — document the new option with a production warning comment

## Security constraints

- Fail-closed behavior must be the default (not opt-in)
- The `fail_open: true` path must log a clear WARN naming the risk
