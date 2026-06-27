# Goals — production-readiness

Drive flint-gate from ~80–85% complete to **100% production-ready** against its own README/docs.

## Objective

Close every documented gap so the codebase matches the behaviour its own documentation promises. Every config knob must do what it says; every advertised feature must work end-to-end.

## Goals

1. **Session watchdog** — enforce mid-stream Kratos session re-validation (currently configured but never applied during streaming). Terminate active streams on session expiry.
2. **WebSocket streaming protocol** — implement real WebSocket passthrough/processing so `stream.protocol: websocket` is no longer silently treated as SSE.
3. **NDJSON streaming protocol** — implement real NDJSON line-delimited passthrough/processing so `stream.protocol: ndjson` is no longer silently treated as SSE.
4. **Redis L2 cache** — implement the documented L2 cache tier (currently explicitly deferred); wire `cache.l2.redis_url` into reads/writes + invalidation.
5. **Route-level `host` filter** — actually check `RouteMatch.host` in `match_route` (currently parsed but never enforced).
6. **AG-UI metadata injection** — resolve `inject_metadata` templates per-stream instead of always passing an empty map.
7. **A2UI theme injection** — wire `_theme` injection in the pipeline (currently always `None`).
8. **K8s readiness probe fix** — correct manifest to use `/ready` on admin port (4457) to match documented contract (currently uses `/health` on proxy port).
9. **JWT signing key rotation** — make the `jwt_signing_keys` table functional (key rotation lifecycle) or remove it if intentionally unused.

## Success Criteria

- `cargo test --workspace` passes with new coverage for every implemented feature.
- `cargo clippy --workspace -- -D warnings` is clean.
- README "Why Flint Gate?" claims all hold true against the running binary.
- No config field that silently does nothing.
