# audit-hot-reload-rollback

**Phase:** beta-release-readiness / Phase 3 (Serious gap S-7)

## Problem

The Cedar policy engine supports hot-reload via `inotify` (or equivalent OS
file-watch). When the config file changes, the engine re-reads it. If the
re-read fails partway through (e.g., the file is truncated mid-write, or a
Cedar policy parse error occurs), it is unclear whether the engine applies
a partial state or rolls back to the last-known-good config.

A partial-apply scenario would be a security incident: some policies apply
from the new config, some from the old, with no deterministic guarantee of
which.

## Audit scope

1. Read `authz/engine.rs` (or wherever hot-reload is implemented)
2. Determine if the reload is atomic: does it swap the entire policy set at
   once, or does it apply changes incrementally?
3. If atomic: add a comment documenting the atomic swap and add a unit test
   that verifies a bad policy file leaves the old set intact
4. If non-atomic: fix to use atomic swap (load new set into a temp, validate,
   then replace the live pointer)

## Expected implementation

The current code likely uses `Arc<RwLock<CedarEngine>>`. The atomic pattern is:
1. Parse new config into a new `CedarEngine` instance (fails fast on error)
2. Only if parse succeeds: `*write_guard = new_engine`
3. On parse error: `warn!` + keep existing engine running

## Files to change

- `authz/engine.rs` (or equivalent) — verify/implement atomic swap
- Unit tests in the same file or `authz/tests.rs` — add rollback test
