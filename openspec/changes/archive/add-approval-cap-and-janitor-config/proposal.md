# add-approval-cap-and-janitor-config

## Goal

G2: Add a configurable max-pending cap to `ApprovalManager` so a burst of
`RequireApproval` decisions under a misconfigured policy cannot grow the
in-memory DashMap unbounded. Also expose the janitor reap interval as a
config field rather than a hardcoded heuristic in `main.rs`.

## Scope

- `crates/flint-gate-core/src/approval/mod.rs` — `ApprovalError::CapExceeded`
  variant; `ApprovalManager::register()` checks `inner.len() >= max_pending`
  before inserting.
- `crates/flint-gate-core/src/config/types.rs` — `ApprovalConfig` gains
  `max_pending: Option<usize>` (default `None` = unbounded, or supply a
  sensible const default of 1000 via serde) and `janitor_interval_seconds:
  Option<u64>`.
- `crates/flint-gate-core/src/middleware/pipeline.rs` — treat `CapExceeded`
  in the `register()` error path the same as any other registration failure:
  fail-closed to Deny (emit deny event, continue stream to termination, no
  panic).
- `crates/flint-gate/src/main.rs` — janitor interval reads
  `approval.janitor_interval_seconds` first, falls back to the existing
  heuristic (`ttl / 2`, min 10s, else 60s).
- `config.example.yaml` — document both new fields.
- Tests: register at cap returns `CapExceeded`; pipeline treats `CapExceeded`
  as deny-not-panic; janitor uses `janitor_interval_seconds` when set.

## Fail-closed invariant

`CapExceeded` MUST deny the tool call (fail-closed). It MUST NOT silently allow
the call through. The test suite must assert this.
