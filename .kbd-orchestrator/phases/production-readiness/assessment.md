# Assessment — production-readiness

**Phase:** production-readiness
**Assessed:** 2026-06-23
**Goal:** Close every documented gap so flint-gate matches its own README/docs — 100% production-ready.

---

## Baseline metrics

| Metric | Value |
|---|---|
| Rust source LOC | 5,305 (22 `.rs` files) |
| Test functions | 66 |
| `TODO` / `FIXME` markers | 0 |
| `todo!()` / `unimplemented!()` macros | 0 |
| Compiles clean | yes |
| `cargo test` | 66 passing |
| `cargo clippy -- -D warnings` | clean |

---

## Gap summary

| # | Goal | Status | Effort | Risk |
|---|---|---|---|---|
| 1 | Session watchdog | **Missing** | Medium | Medium |
| 2 | WebSocket streaming | **Missing** | High | Medium |
| 3 | NDJSON streaming | **Missing** | Medium | Low |
| 4 | Redis L2 cache | **Missing** | Medium | Low |
| 5 | Route-level `host` filter | **Missing** | Low | Low |
| 6 | AG-UI metadata injection | **Partial** | Low | Low |
| 7 | A2UI theme injection | **Partial** | Low | Low |
| 8 | K8s readiness probe | **Partial** | Trivial | None |
| 9 | JWT signing keys table | **Missing** | Medium | Low |

**Legend:** Missing = config-only or dead code. Partial = machinery exists, pipeline doesn't wire it.

---

## Goal-by-goal evidence

### 1. Session watchdog — MISSING

- **Parsed, never read.** `SessionWatchdogConfig` defined `src/config/types.rs:451-457`; field at `types.rs:421`. A grep for `session_watchdog` across `src/` returns only those two type definitions — the field is never consumed.
- **Streaming enforcement absent.** `SseStreamProcessor` (`src/stream/processor.rs:55-250`) only enforces the two backpressure checks (duration `processor.rs:93-99`, event-count `processor.rs:102-108`). There is no credential re-validation loop and no field to hold a watchdog handle.
- **Spawn site.** Processor is constructed in `src/middleware/pipeline.rs:427` with no authenticator or credential passed in.
- **To close:** add a watcher task (`tokio::time::interval` at `check_interval_seconds`) that re-validates the Kratos session; on failure signal the processor via `CancellationToken` and emit a terminal `RUN_ERROR` (same path as backpressure at `processor.rs:97`).

### 2. WebSocket streaming — MISSING

- **No dispatch point.** `StreamConfig.protocol` exists (`src/config/types.rs:404-410`, default `"sse"`) but `src/middleware/pipeline.rs:427` unconditionally constructs `SseStreamProcessor::new(...)` with no `match` on protocol.
- **`axum` `ws` feature is enabled** (`Cargo.toml:21`) so the dependency surface is ready, but no `WebSocketUpgrade` handler exists and the catch-all `proxy_handler` (`pipeline.rs:64`) has no WS branch.
- **To close:** introduce a `StreamProcessor` trait; implement `WsStreamProcessor` + an axum `WebSocketUpgrade` route (or upstream-bridge via `reqwest` upgrade). Dispatch in `pipeline.rs` between lines 417 and 427.

### 3. NDJSON streaming — MISSING

- **Same missing dispatch as Goal 2.** `"ndjson"` is enumerated in the `protocol` doc comment (`types.rs:403`) but never branched on.
- **Wire format incompatible.** `SseStreamProcessor::process_line` (`processor.rs:143-171`) is hard-bound to SSE framing (`data:`, `event:`, blank-line terminator). NDJSON is one JSON object per `\n` with no prefix.
- **To close:** implement `NdjsonStreamProcessor` (split on `\n`, parse each line as JSON) and dispatch from the same `match` added in Goal 2.

### 4. Redis L2 cache — MISSING

- **Config present.** `L2CacheConfig { enabled, redis_url }` at `src/config/types.rs:152-158`; nested under `CacheConfig.l2` at `types.rs:107`.
- **README admits it:** `README.md:198` — *"Redis L2 (not yet implemented)"*.
- **Zero implementation.** `GateCache` (`src/cache/mod.rs:17-114`) holds only three `moka::future::Cache` fields; `from_config` (`mod.rs:29-49`) ignores `cfg.l2`. **No `redis` crate in `Cargo.toml`** and no `redis::` references anywhere in `src/`.
- **To close:** add `redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }`; add `Option<redis::aio::ConnectionManager>` to `GateCache`; implement read-through in `get_session` (`mod.rs:68-81`), write-through in `put_session` (`mod.rs:84-90`), and DEL/scan in `invalidate_session`/`invalidate_all` (`mod.rs:93-97, 52-57`). Decide key prefix (`flint:session:<hash>`).

### 5. Route-level `host` filter — MISSING

- **Parsed, never checked.** `RouteMatch.host: Option<String>` at `src/config/types.rs:315-316`.
- **`match_route` ignores it.** `src/proxy/router.rs:138-181` checks only the *site*-level domain list (`router.rs:146-150`). The `host` arg is logged (`router.rs:173`) but never compared to `route_match.host`. Existing tests set `host: None` (`router.rs:304, 417, 470`).
- **To close:** insert a route-level host check inside the loop at `router.rs:144` (between site check and path check); implement `host_matches` (exact + `*.example.com` wildcard — `glob_to_regex` at `router.rs:221` is path-oriented and would need a hostname-aware variant).

### 6. AG-UI metadata injection — PARTIAL

- **Mechanism complete, pipeline drops it.** `AgUiConfig.inject_metadata: HashMap<String,String>` at `types.rs:436-438`; `AgUiProcessor::process` accepts `metadata: serde_json::Map` (`ag_ui.rs:143-147`) and merges under `_gate_metadata` (`ag_ui.rs:66-70`).
- **Hard-coded empty map.** `processor.rs:201` — `let meta = serde_json::Map::new();`. The processor is constructed at `processor.rs:58-65` and `inject_metadata` is dropped.
- **To close:** in `pipeline.rs` around line 427, render each `inject_metadata` entry via `TemplateEngine::render` against the in-scope `template_ctx` (built at `pipeline.rs:225`), convert to `serde_json::Map` (parse-as-JSON-else-string, same as `apply_body_transforms` `pipeline.rs:518-525`), pass into `SseStreamProcessor::new` as a new param, and replace the literal at `processor.rs:201`.

### 7. A2UI theme injection — PARTIAL

- **Mechanism complete, pipeline passes `None`.** `A2UiProcessor::process` accepts `theme: Option<Value>` (`a2ui.rs:85`) and calls `inject_theme` (`a2ui.rs:107-109`). The call site is `processor.rs:219` — `a2ui_proc.process(event, &self.user_scopes, None)`.
- **No config field.** `A2UiConfig` (`types.rs:442-448`) has only `enabled` and `allowed_intents` — nowhere to declare a theme.
- **To close:** add `theme: Option<serde_json::Value>` to `A2UiConfig`; thread it through `SseStreamProcessor::new` alongside the Goal 6 change; replace literal `None` at `processor.rs:219`.

### 8. K8s readiness probe — PARTIAL (trivial fix)

- **Manifest mis-configured.** `k8s/deployment.yaml:58-64` sets `readinessProbe.httpGet.path: /health`, `port: proxy` (4456).
- **Correct endpoint exists.** `/ready` is wired on the admin router at `src/admin/mod.rs:47` and `ready_handler` (`admin/mod.rs:68-81`) actually probes Postgres with `SELECT 1`, returning 503 when the DB is down. Admin server binds `4457`.
- **To close:** change `deployment.yaml:58-64` to `path: /ready`, `port: admin`. No Rust change.

### 9. JWT signing keys table — MISSING

- **Table created, never used.** `CREATE TABLE jwt_signing_keys` at `src/db/mod.rs:54-61` inside `SCHEMA_SQL`. A grep for `jwt_signing_keys` across `src/` returns **only that one line** — no SELECT/INSERT/UPDATE/DELETE.
- **Actual signing uses config values.** `JwtConfig.signing_key_path` / `signing_key_secret` (`types.rs:243-245`) → loaded `main.rs:81-82,170` → consumed by `jwt_mint.rs:33,36,46,48,61,63`. None of this touches the DB.
- **README marks it future:** `README.md:656`.
- **To close (decision required):**
  - **Option A — implement rotation:** add `Database::{get_active_signing_key, list_signing_keys, insert_signing_key, deactivate_signing_key}`, prefer DB-sourced key in `JwtMinter`, add `/signing-keys` admin endpoints, hook NOTIFY-based reload.
  - **Option B — remove the table:** delete `db/mod.rs:54-61` and the README line. Cheapest; `CREATE TABLE IF NOT EXISTS` makes re-adding safe later.

---

## Architectural blocker (affects Goals 1, 2, 3, 6, 7)

`SseStreamProcessor` is both the **wire-format framer** and the **event-processing chain** (AG-UI / A2UI / backpressure). It is the only stream processor. Closing the streaming-related goals cleanly requires either:

- **(A)** Extract a `StreamProcessor` trait with per-protocol framer impls (`SseProcessor`, `WsProcessor`, `NdjsonProcessor`) sharing a common event-processor chain; or
- **(B)** Split `SseStreamProcessor` into a framer + a pluggable event-processor chain, leaving SSE as the default framer.

Option A is more work but is the right long-term shape and unblocks WebSocket/NDJSON dispatch in one move. **Recommend sequencing Goal 2's trait extraction first**, then implementing 3, 6, 7 against it, then 1 (watchdog) which is agnostic to framer.

---

## Recommended sequencing (for /kbd-analyze or /kbd-plan)

| Wave | Goals | Rationale |
|---|---|---|
| 1 | 8 | Trivial, no code risk, ships value immediately. |
| 2 | 5 | Small, isolated, builds test coverage for matching. |
| 3 | 6 + 7 | Same fix point (`processor.rs:201,219` + `SseStreamProcessor::new`); do together. |
| 4 | 9 (decide A vs B) | Decide implement-vs-remove; unblocks cleanup. |
| 5 | 4 | Self-contained tier; touches only `cache/mod.rs` + `Cargo.toml`. |
| 6 | 2 (+ trait extraction) | Architectural pivot; do before 3 and 1. |
| 7 | 3 | Builds on the trait from wave 6. |
| 8 | 1 | Watchdog; agnostic to framer but benefits from the trait. |

---

## Open questions for /kbd-analyze or /kbd-plan

1. **Goal 9 — implement or remove?** Is JWT key rotation on the near-term roadmap, or is the table speculative? (Recommend: implement if any multi-tenant signing need exists; otherwise remove and re-add later.)
2. **Goal 2 — transparent WS bridge or terminate-and-replay?** A transparent bidirectional byte bridge is simpler but can't run AG-UI/A2UI/backpressure on WS frames. Terminating at flint-gate enables metering but changes the contract. Which does the product need?
3. **Goal 4 — Redis Pub/Sub or rely on existing Postgres LISTEN/NOTIFY?** All instances already receive PG notifications; adding Redis Pub/Sub is redundant unless Redis is the only shared state.
4. **Goal 1 — watchdog trip semantics.** On session expiry mid-stream: emit `RUN_ERROR` and close, or attempt a graceful 401 mid-stream? README says "terminate" but doesn't specify the wire behaviour.

---

## Stage gate

Assess is the first stage — gate passes. Handoff to next stage (analyze or plan):

> Nine documented gaps confirmed with file:line evidence. One architectural blocker (single hard-coded SSE processor) affects five goals — recommend extracting a `StreamProcessor` trait before WebSocket/NDJSON/watchdog work. Two goals (6, 7) share a single fix point and should be done together. One goal (8) is a one-line manifest fix. One goal (9) needs an implement-vs-remove decision before planning. Recommended sequencing in 8 waves above.
