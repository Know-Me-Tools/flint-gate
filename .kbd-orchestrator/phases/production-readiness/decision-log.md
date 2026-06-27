# Decision Log — production-readiness

| ID | Date | Decision | Rationale | Confidence |
|---|---|---|---|---|
| D1 | 2026-06-23 | ADOPT `tokio-tungstenite` 0.28 for WebSocket (Goal 2) | Already in lockfile (0.28.0) via axum ws feature. De facto standard. Promote to direct dep. | High |
| D2 | 2026-06-23 | ADOPT `redis` 1.x for L2 cache (Goal 4) | Simplest fit for GET/SET/DEL L2 tier. ConnectionManager pools natively + reconnects. | High |
| D3 | 2026-06-23 | SKIP `fred` 10.x | Overkill — cluster/pubsub/scripting/Valkey for a 3-key-type L2 cache. | High |
| D4 | 2026-06-23 | SKIP `deadpool-redis` 0.23 | Redundant pool layer; redis-rs ConnectionManager already pools. | High |
| D5 | 2026-06-23 | BUILD streaming trait abstraction | No crate fits; standard Rust trait + composition; unblocks Goals 1,2,3,6,7. | High |
| D6 | 2026-06-23 | Rely on Postgres LISTEN/NOTIFY for cache invalidation (not Redis Pub/Sub) | Already reaches all instances; Redis is data-only L2, not a message bus. | High |
| D7 | 2026-06-23 | BUILD host matcher (not reuse `glob_to_regex`) | Path-oriented regex wrong for hostnames; dedicated 10-line `host_matches`. | High |
