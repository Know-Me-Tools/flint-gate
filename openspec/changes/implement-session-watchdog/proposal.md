# implement-session-watchdog

## Summary
Implement mid-stream session re-validation (session watchdog) that terminates active streams when a Kratos session expires during streaming.

## Motivation
`SessionWatchdogConfig { enabled, check_interval_seconds }` is defined at `src/config/types.rs:451-457` and parsed on `AiStreamConfig` (`types.rs:421`), but it is never read after parsing. `SseStreamProcessor` only enforces backpressure (duration + event count) — there is no credential re-validation loop. The README claims "Session watchdog terminates active streams on expiry" (`README.md:22`) but this is not implemented.

## Design
1. **WatchdogHandle**: A struct holding a `CancellationToken`, the credential to re-validate, and a reference to the authenticator + cache.

2. **Watchdog task**: When `session_watchdog.enabled`, spawn a `tokio::time::interval(check_interval_seconds)` task that:
   - Calls the Kratos authenticator's `validate_session(credential)` (re-fetches `/sessions/whoami`).
   - On session still valid: continue.
   - On session expired: fire the `CancellationToken`, which causes the stream processor to stop processing and emit the protocol-appropriate error.

3. **Per-protocol error on watchdog trip** (depends on `StreamProcessor` trait from change #6):
   - SSE: emit `RUN_ERROR` event with reason `"session_expired"`, then close.
   - WS: send close frame with code 1008 (policy violation).
   - NDJSON: append `{"error":"session_expired"}` line, then close.

4. **Plumbing**: Extend `SseStreamProcessor::new` (or the trait's constructor pattern) to accept `credential: Option<String>` and `authenticator: Option<Arc<dyn Authenticator>>`. The watchdog is constructed inside `new` when `session_watchdog.enabled`.

5. **Cache interaction**: On watchdog trip, call `state.cache.invalidate_session(credential)` so subsequent requests with the expired credential are not served from cache.

## Depends on
- `extract-stream-processor-trait` (change #6) — provides the trait abstraction so the watchdog works across SSE/WS/NDJSON framers.

## Tasks
- [ ] Define `WatchdogHandle` struct (CancellationToken + credential + authenticator ref)
- [ ] Implement watchdog interval task (re-validate Kratos session at check_interval_seconds)
- [ ] On session expiry: fire CancellationToken to terminate stream
- [ ] Implement per-protocol error emission (SSE RUN_ERROR, WS close 1008, NDJSON error line)
- [ ] Plumb credential + authenticator through stream processor constructors
- [ ] Call `cache.invalidate_session(credential)` on watchdog trip
- [ ] Add tests: watchdog fires on expiry, watchdog passes when session valid, cancellation cleanup
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
