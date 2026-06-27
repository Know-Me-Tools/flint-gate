# Plan — production-readiness

**Phase:** production-readiness
**Planned:** 2026-06-23
**Inputs:** `assessment.md`, `analysis.md`, `library-candidates.json`
**Change backend:** OpenSpec (`openspec/changes/`)

---

## Resolved decisions (from analysis open questions)

| Question | Decision | Rationale |
|---|---|---|
| WS: transparent bridge vs terminate-and-replay? | **Terminate-and-replay** | Only way to honor README's "AG-UI processing on streams" for WS. Metering/filtering requires frame inspection. |
| JWT keys: implement or remove? | **Implement rotation** | Table + DDL exist; `/api-keys` admin is a copy-paste template; rotation is a real production need. |
| Watchdog trip wire format? | **Per-protocol:** SSE → `RUN_ERROR` event + close; WS → close frame 1008 (policy violation); NDJSON → `{"error":"session_expired"}` line + close. | Matches each protocol's error semantics. |

---

## Ordered change list (8 changes)

Changes are ordered by dependency: trivial fixes first, shared fix points next, architectural pivot before protocol impls, watchdog last (benefits from trait).

| # | Change ID | Goals | Deps | Library | Agent |
|---|---|---|---|---|---|
| 1 | `fix-k8s-readiness-probe` | 8 | none | — | codex |
| 2 | `implement-route-host-filter` | 5 | none | — | codex |
| 3 | `wire-stream-metadata-injection` | 6, 7 | none | — | codex |
| 4 | `implement-jwt-key-rotation` | 9 | none | — | codex |
| 5 | `implement-redis-l2-cache` | 4 | none | `redis` 1.x (adopt) | codex |
| 6 | `extract-stream-processor-trait` | 2 + arch | none | `tokio-tungstenite` 0.28 (adopt, already in lockfile) | codex |
| 7 | `implement-ndjson-streaming` | 3 | #6 | — | codex |
| 8 | `implement-session-watchdog` | 1 | #6 | — | codex |

### Ordering rationale

1. **#1 (K8s probe)** — trivial one-line YAML fix, zero code risk, ships immediate operational value.
2. **#2 (host filter)** — small isolated change in `router.rs`, builds test coverage for matching.
3. **#3 (metadata + theme)** — shared fix point at `processor.rs:201,219`; goals 6+7 done together because they touch the same 3 lines.
4. **#4 (JWT rotation)** — self-contained DB + admin work; no dependency on streaming changes.
5. **#5 (Redis L2)** — self-contained cache tier; touches only `cache/mod.rs` + `Cargo.toml`.
6. **#6 (trait + WebSocket)** — architectural pivot. Extract `StreamProcessor` trait, refactor `SseStreamProcessor` → `SseFramer`, implement `WsFramer` with `tokio-tungstenite`. Must land before #7 and #8.
7. **#7 (NDJSON)** — builds on the trait from #6; small new framer impl.
8. **#8 (watchdog)** — agnostic to framer but benefits from trait; last because it touches the streaming task lifecycle and is the highest-risk change.

---

## Library annotations

| Change | Verdict | Crate | Evidence |
|---|---|---|---|
| #5 `implement-redis-l2-cache` | adopt | `redis` 1.x | `library-candidates.json` D2 — simplest fit, ConnectionManager pools natively |
| #6 `extract-stream-processor-trait` | adopt | `tokio-tungstenite` 0.28 | `library-candidates.json` D1 — already in lockfile via axum ws |

All other changes are BUILD — no external library adoption.

---

## Per-change summaries

### Change 1: `fix-k8s-readiness-probe`
Fix `k8s/deployment.yaml` readiness probe to use `/ready` on admin port (4457) instead of `/health` on proxy port. The `/ready` endpoint (`admin/mod.rs:47,68-81`) already probes Postgres connectivity. One-line YAML edit, no Rust change.

### Change 2: `implement-route-host-filter`
Add `host_matches()` helper in `router.rs` (exact + `*.example.com` suffix). Insert route-level host check in `match_route` between site check (`router.rs:146`) and path check (`router.rs:153`). Strip port from Host header before matching. Add unit tests for exact, wildcard, and mismatch cases.

### Change 3: `wire-stream-metadata-injection`
**Goal 6 (AG-UI metadata):** In `pipeline.rs` around line 427, render each `inject_metadata` template via `TemplateEngine::render` against `template_ctx`; convert `HashMap<String,String>` → `serde_json::Map` (parse-as-JSON-else-string); pass into `SseStreamProcessor::new`; replace literal `serde_json::Map::new()` at `processor.rs:201`.
**Goal 7 (A2UI theme):** Add `theme: Option<serde_json::Value>` to `A2UiConfig`; thread through `SseStreamProcessor::new`; replace literal `None` at `processor.rs:219`.

### Change 4: `implement-jwt-key-rotation`
Add `Database` methods: `get_active_signing_key()`, `list_signing_keys()`, `insert_signing_key()`, `deactivate_signing_key(id)`. In `JwtMinter`, prefer DB-sourced active key over config file/secret; fall back to config when DB is unconfigured or table empty. Add admin endpoints under `/signing-keys` mirroring `/api-keys` pattern. Hook cache invalidation so key rotation NOTIFY forces minter reload.

### Change 5: `implement-redis-l2-cache`
Add `redis = { version = "1", features = ["tokio-comp", "connection-manager"] }` to `Cargo.toml`. Add `Option<redis::aio::ConnectionManager>` to `GateCache`. Connect in `from_config` when `l2.enabled`. Implement read-through in `get_session`, write-through in `put_session`, DEL in `invalidate_session`, SCAN+DEL prefix in `invalidate_all`. Key scheme: `flint:session:<hash>`, `flint:route:<id>`, `flint:kv:<key>`. Rely on existing Postgres LISTEN/NOTIFY for cross-instance invalidation (Redis is data-only).

### Change 6: `extract-stream-processor-trait`
Define `StreamProcessor` trait (`process_chunk`, `flush_event`, `metrics`). Refactor `SseStreamProcessor` into `SseFramer` (wire-format logic moves here) + shared `EventChain` (AG-UI, A2UI, backpressure). Implement `WsFramer` using `tokio-tungstenite::connect_async` for upstream WS bridging. Add protocol dispatch in `pipeline.rs` (match on `stream_config.protocol`). WS uses terminate-and-replay: inspect Text frames for AG-UI JSON, apply event chain. Promote `tokio-tungstenite` from transitive to direct dep in `Cargo.toml`.

### Change 7: `implement-ndjson-streaming`
Implement `NdjsonFramer` implementing `StreamProcessor` trait. Split response body on `\n`, parse each non-empty line as JSON via `serde_json::from_str`, run through `EventChain`. Set `Content-Type: application/x-ndjson`. Dispatch from the `match` added in change #6. Per-protocol error shape: `{"error":"..."}` line + close.

### Change 8: `implement-session-watchdog`
Add `watchdog: Option<WatchdogHandle>` to stream processors. When `session_watchdog.enabled`, spawn a `tokio::time::interval` task at `check_interval_seconds` that re-validates the Kratos session via the authenticator. On session expiry, signal the processor via `CancellationToken`; processor emits the protocol-appropriate error (SSE: `RUN_ERROR`, WS: close 1008, NDJSON: error JSON line) and terminates the stream. Plumb credential + authenticator through `SseStreamProcessor::new` (currently only passes `stream_config` + `user_scopes`).

---

## Execution handoff

> 8 ordered changes. Change #1 is a one-line YAML fix — start there. Changes #1–#5 are independent and can be parallelized across agents if desired. Change #6 is the architectural gate: #7 and #8 depend on the `StreamProcessor` trait it introduces. Recommended agent: codex for all changes (Rust-native, compile-check loop). Verify after each change: `cargo test --workspace && cargo clippy --workspace -- -D warnings`.
