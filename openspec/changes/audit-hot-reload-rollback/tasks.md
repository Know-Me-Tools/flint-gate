- [x] Read `authz/engine.rs` (and any hot-reload watcher code) to determine current reload strategy
- [x] Confirm whether reload is atomic (full swap) or incremental (partial apply)
      FINDING: Engine uses `ArcSwap` (not `Arc<RwLock>`) — both reload paths parse-before-swap.
      `reload_from_records` and `reload_from_records_lenient` build new bundle in local value first,
      then call `ArcSwap::store()` — single atomic pointer replacement, no partial state possible.
- [x] If incremental: refactor to load new engine into temp, validate, then swap `Arc<RwLock<_>>` pointer atomically
      N/A — already atomic.
- [x] If already atomic: add doc comment on the reload function documenting the atomic-swap guarantee
      Added comprehensive hot-reload atomicity guarantee doc comment to `AuthzEngine` struct.
- [x] Add unit test: inject a bad policy file mid-reload; assert existing engine still returns correct decisions
      Already exists: `reload_retains_last_good_on_bad_policy` (line 700) — passes.
- [x] Add unit test: inject a valid new policy file; assert new decisions reflect new policy after reload
      Already exists: `reload_swaps_in_new_good_bundle` (line 725) — passes.
- [x] `cargo test --workspace` passes — 539 tests passed, both rollback tests verified
