# Analysis — production-readiness

**Phase:** production-readiness
**Analyzed:** 2026-06-23
**Mode:** Stack-specified (Rust / Axum 0.8 / Tokio)
**Inputs:** `assessment.md` (9 confirmed gaps with file:line evidence)

---

## Landscape summary

Of the 9 assessed gaps, only **two** require external research:

| Gap | Needs external dep? | Verdict |
|---|---|---|
| 1 — Session watchdog | No | BUILD (tokio patterns already in tree) |
| 2 — WebSocket streaming | **Yes** | ADOPT `tokio-tungstenite` (**already in lockfile**) |
| 3 — NDJSON streaming | No | BUILD (line-split + serde_json) |
| 4 — Redis L2 cache | **Yes** | ADOPT `redis` crate (1.x) |
| 5 — Route-level host filter | No | BUILD (string match) |
| 6 — AG-UI metadata injection | No | BUILD (existing TemplateEngine) |
| 7 — A2UI theme injection | No | BUILD (existing TemplateEngine) |
| 8 — K8s readiness probe | No | BUILD (YAML edit) |
| 9 — JWT signing keys table | No | BUILD (sqlx queries or DDL deletion) |

**Build-vs-adopt ratio: 7 build, 2 adopt (one of which is already satisfied).** This phase is mostly internal plumbing, not library integration.

---

## Candidate evaluation

### Gap 2 — WebSocket streaming

**Decision: ADOPT `tokio-tungstenite` (already in dependency tree)**

| Source | Finding |
|---|---|
| `Cargo.lock` | `tokio-tungstenite` 0.28.0 present, pulled by `axum` `ws` feature (`Cargo.toml:21`). `tungstenite` 0.28.0 also present. |
| `cargo search` | Latest: `tokio-tungstenite = "0.29.0"`. Lockfile has 0.28.0 (axum 0.8.8 constrains it). Pin to `0.28` to match axum's constraint; do not force 0.29. |

**Why not alternatives:**
- `axum::extract::ws` — already enabled, but it's for *inbound* WS upgrades (server-side). For **upstream** WS bridging (flint-gate as WS client to backend), we need `tokio-tungstenite::connect_async()` directly. The axum inbound handler + tungstenite outbound client is the correct pairing.
- `ws-bridge` (0.2.1) — niche, strongly-typed endpoint macro; wrong shape for a transparent proxy.
- `atomic_websocket` (0.8.0) — high-level util; overkill.

**Integration shape:**
1. Add `tokio-tungstenite = { version = "0.28", features = ["rustls-tls-webpki-roots"] }` as a **direct** dependency (it's already transitively present; promoting it avoids relying on axum's internal feature surface).
2. In `pipeline.rs`, detect `Upgrade: websocket` headers before the SSE path; branch to a `WsStreamProcessor` that calls `tokio_tungstenite::connect_async(upstream_url)` and pipes `Message::Text` / `Message::Binary` bidirectionally.
3. AG-UI/A2UI event filtering can run on WS `Text` frames that contain AG-UI JSON (same parse path as `ag_ui.rs:143`).

**Confidence: High.** Crate is already compiled into the binary; it's the de facto Rust WS standard.

---

### Gap 4 — Redis L2 cache

**Decision: ADOPT `redis` crate (redis-rs) at 1.x**

| Source | Finding |
|---|---|
| `cargo search redis` | `redis = "1.2.4"` — latest stable. |
| `cargo search fred` | `fred = "10.1.0"` — full-featured cluster client. |
| `cargo search deadpool-redis` | `deadpool-redis = "0.23.0"` — pool wrapper. |

**Candidate ranking:**

| Crate | Version | Fit | Verdict |
|---|---|---|---|
| **redis (redis-rs)** | 1.2.4 | Simple GET/SET/DEL + built-in `ConnectionManager` | **ADOPT** — minimal surface, matches the trivial L2 KV use case |
| fred | 10.1.0 | Cluster, pub/sub, scripting, Valkey | Skip — overkill for a 3-key-type L2 tier (sessions/routes/kv) |
| deadpool-redis | 0.23.0 | Pool on top of redis-rs | Skip — redis-rs `ConnectionManager` already pools; deadpool adds a layer with no benefit for this read-through/write-through pattern |

**Features needed:** `["tokio-comp", "connection-manager"]` — async runtime compat + reconnecting pooled manager.

**Integration shape:**
1. `Cargo.toml`: `redis = { version = "1", features = ["tokio-comp", "connection-manager"] }`.
2. `GateCache` (`cache/mod.rs:17`): add `l2: Option<redis::aio::ConnectionManager>`.
3. `from_config` (`mod.rs:29`): when `cfg.l2.enabled && cfg.l2.redis_url.is_some()`, `redis::Client::open(url)?.get_connection_manager().await`.
4. Read-through in `get_session` (`mod.rs:68`): L1 miss → `GET flint:session:<hash>` → backfill L1.
5. Write-through in `put_session` (`mod.rs:84`): `SET flint:session:<hash> <val> EX <ttl>`.
6. Invalidation: `invalidate_session` (`mod.rs:93`) → `DEL`; `invalidate_all` (`mod.rs:52`) → SCAN + DEL by prefix `flint:*`.

**Invalidation bus:** the existing Postgres LISTEN/NOTIFY (`mod.rs:130-179`) already reaches all instances. Redis Pub/Sub would be redundant. **Decision: do not add Redis Pub/Sub; rely on the existing PG channel for cross-instance invalidation.** Redis is purely a shared L2 data tier, not a message bus.

**Confidence: High.** redis-rs is the standard Rust Redis client; the use case is textbook GET/SET/DEL.

---

### Gaps with no external dependency (BUILD)

| Gap | Approach | Existing code to reuse |
|---|---|---|
| 1 — Session watchdog | `tokio::time::interval` + `CancellationToken` (both already deps) re-validating Kratos session; on expiry, cancel the stream processor | Backpressure cancel path at `processor.rs:97,106`; Kratos authenticator at `auth/kratos.rs` |
| 3 — NDJSON | Split response body on `\n`, parse each line with `serde_json::from_str`, run the same AG-UI/A2UI event chain | `serde_json` (dep), `ag_ui.rs:143` event parse, `a2ui.rs:85` intent filter |
| 5 — Host filter | Exact match + `*.example.com` suffix match against `Host` header (strip port first) | `glob_to_regex` at `router.rs:221` is path-oriented — do **not** reuse for hostnames; write a dedicated 10-line `host_matches` |
| 6 — AG-UI metadata | Render `inject_metadata` templates via existing `TemplateEngine::render` against per-request `template_ctx` | `TemplateEngine` at `config/template.rs`; `template_ctx` built at `pipeline.rs:225`; `apply_body_transforms` JSON-parse pattern at `pipeline.rs:518-525` |
| 7 — A2UI theme | Add `theme: Option<Value>` to `A2UiConfig`; pass through `SseStreamProcessor::new`; replace literal `None` at `processor.rs:219` | `A2UiProcessor::process` already accepts `theme: Option<Value>` (`a2ui.rs:85`) — mechanism is complete |
| 8 — K8s probe | Change `k8s/deployment.yaml:58-64`: `path: /ready`, `port: admin` | `/ready` endpoint already exists at `admin/mod.rs:47,68-81` |
| 9 — JWT signing keys | **Decision deferred to plan** — either implement CRUD + rotation (`sqlx` queries against existing table) or delete the `CREATE TABLE` at `db/mod.rs:54-61` | `sqlx` (dep); existing `/api-keys` admin pattern at `admin/mod.rs:219-318` as a template if implementing |

---

## Architectural recommendation (streaming abstraction)

The assessment flagged that `SseStreamProcessor` is hard-bound to SSE framing and blocks Goals 1, 2, 3, 6, 7. The clean shape:

```
StreamProcessor (trait)
├── fn process_chunk(&mut self, bytes: Bytes) -> Option<Vec<u8>>
├── fn flush_event(&mut self) -> Option<Vec<u8>>
└── fn metrics(&self) -> &StreamMetrics

SseFramer implements StreamProcessor   // data:/event:/blank-line — existing logic
NdjsonFramer implements StreamProcessor // \n-delimited JSON — new
WsFramer implements StreamProcessor     // WS Text/Binary frames — new

EventChain (composition, not per-framer)
├── AgUiProcessor (validate + meter + metadata)
├── A2UiProcessor (intent filter + scope + theme)
├── Watchdog (session re-validation)
└── Backpressure (duration + event count)
```

**Why a trait, not an enum:** the three framers share zero wire-format logic but share the entire event-processing chain. A trait lets the chain be composed once and applied to any framer. An enum would duplicate the chain dispatch or require a macro.

**Effort estimate:** ~300 LOC for the trait + refactor of existing `SseStreamProcessor` into `SseFramer` + `EventChain`. The refactor is mechanical (move `process_line` body into `SseFramer`, move AG-UI/A2UI/backpressure into `EventChain`). Net new logic for NDJSON/WS is small.

**No external crate needed for the trait itself** — this is a standard Rust trait + composition pattern. `async-trait` is already a dependency (`Cargo.toml:74`) if async methods are needed, but the processors are synchronous over bytes so plain traits likely suffice.

---

## Decision log entries

| ID | Decision | Rationale | Confidence |
|---|---|---|---|
| D1 | ADOPT `tokio-tungstenite` 0.28 for WebSocket | Already in lockfile via axum ws; de facto standard | High |
| D2 | ADOPT `redis` 1.x for L2 cache | Simplest fit; ConnectionManager pools natively | High |
| D3 | SKIP `fred` | Overkill (cluster/pubsub/scripting) for 3-key-type L2 | High |
| D4 | SKIP `deadpool-redis` | Redundant with redis-rs ConnectionManager | High |
| D5 | BUILD streaming trait abstraction | No crate fits; standard Rust pattern; unblocks 5 goals | High |
| D6 | Rely on Postgres LISTEN/NOTIFY for invalidation (not Redis Pub/Sub) | Already reaches all instances; Redis is data-only | High |
| D7 | BUILD host matcher (not reuse `glob_to_regex`) | Path-oriented regex wrong for hostnames; 10-line helper | High |

---

## Open questions for /kbd-plan

1. **Goal 2 — transparent WS bridge vs terminate-and-replay.** A transparent byte bridge (pipe WS frames both directions without inspecting) is simple but can't meter/filter. Terminating at flint-gate enables AG-UI/A2UI on WS frames but adds a re-framing step. **Recommendation: terminate-and-replay** — it's the only way to honor the README's "AG-UI event processing on streams" claim for WS. Confirm in plan.
2. **Goal 9 — implement JWT rotation or remove the table.** Assessment offers both. Analyze has no preference — this is a product roadmap question. **Recommendation: implement** (the table + DDL exist, the `/api-keys` admin pattern is a copy-paste template, and rotation is a real production need). Escalate to user if contested.
3. **Goal 1 — watchdog trip wire format.** Emit `RUN_ERROR` SSE event and close, or send a 401 status mid-stream? For SSE this is clear (terminal event + close). For WS, there's no standard "error frame" — likely close with code 1008 (policy violation). For NDJSON, append `{"error":"session_expired"}` line. Confirm per-protocol error shape in plan.

No contested choices. No `/pmpo-elicit` escalation needed.
