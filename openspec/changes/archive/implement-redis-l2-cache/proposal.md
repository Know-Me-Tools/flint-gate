# implement-redis-l2-cache

## Summary
Implement the Redis L2 cache tier using the `redis` crate (redis-rs 1.x) with read-through, write-through, and prefix-based invalidation.

## Motivation
`L2CacheConfig { enabled, redis_url }` is defined at `src/config/types.rs:152-158` but `GateCache` (`src/cache/mod.rs:17-114`) only has moka L1 caches. The README explicitly says "not yet implemented" (`README.md:198`). Multi-instance deployments need a shared cache tier to avoid each pod independently querying Postgres/Kratos for the same session.

**Library: adopt `redis` 1.x** (`library-candidates.json` D2). Features: `["tokio-comp", "connection-manager"]`.

## Design
1. **Cargo.toml**: Add `redis = { version = "1", features = ["tokio-comp", "connection-manager"] }`.

2. **GateCache** (`src/cache/mod.rs`):
   - Add `l2: Option<redis::aio::ConnectionManager>` field.
   - In `from_config` (`mod.rs:29`): when `cfg.l2.enabled && cfg.l2.redis_url.is_some()`, open `redis::Client::open(url)?` and `get_connection_manager().await`.

3. **Key scheme**: `flint:session:<sha256_hash>`, `flint:route:<id>`, `flint:kv:<key>`.

4. **Read-through** in `get_session` (`mod.rs:68-81`):
   - L1 miss → `GET flint:session:<hash>` from Redis.
   - On L2 hit → deserialize, back-fill L1, return.
   - On L2 miss → return `None` (caller queries source and writes through).

5. **Write-through** in `put_session` (`mod.rs:84-90`):
   - Write to L1 (moka) AND L2 (`SET flint:session:<hash> <val> EX <ttl>`).

6. **Invalidation**:
   - `invalidate_session` (`mod.rs:93-97`): `DEL flint:session:<hash>` on L2 + L1.
   - `invalidate_all` (`mod.rs:52-57`): L1 clear + `SCAN` + `DEL` by `flint:*` prefix on L2.

7. **Cross-instance invalidation**: Rely on the **existing** Postgres LISTEN/NOTIFY (`mod.rs:130-179`). Redis is data-only L2, not a message bus. All instances already receive PG notifications.

## Tasks
- [ ] Add `redis = { version = "1", features = ["tokio-comp", "connection-manager"] }` to Cargo.toml
- [ ] Add `l2: Option<redis::aio::ConnectionManager>` to GateCache
- [ ] Connect ConnectionManager in `from_config` when `l2.enabled`
- [ ] Implement read-through in `get_session` (L1 miss → L2 GET → back-fill L1)
- [ ] Implement write-through in `put_session` (L1 + L2 SET with TTL)
- [ ] Implement `DEL` in `invalidate_session` and SCAN+DEL prefix in `invalidate_all`
- [ ] Update README: remove "not yet implemented" note for `cache.l2`
- [ ] Add tests: L2 read-through, write-through, invalidation, fallback when L2 disabled
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
