# add-budget-rate-limiting

## Summary
Extend token metering into enforced, windowed token budgets and add request-rate limiting — per-key/per-team, blocking on threshold. (Goal G3)

## Design
Extend `MaxTokenBudgetConfig` beyond the lifetime cap with `window` (minute|hour|day) and `scope` (key|team). Add an in-process request-rate layer via `governor` + `tower_governor` on the proxy router (per-replica burst shield). Add authoritative, shared enforcement via hand-rolled Redis Lua window counters behind the existing `redis-l2` feature (fixed/sliding window `INCR`+`EXPIRE` / sorted-set), keyed `budget:{key}:{window}` and `ratelimit:{key}:{window}`. Keep Postgres `usage_events` as the durable ledger. When `redis-l2` is off, fall back to governor-only rate limiting + a briefly-cached Postgres windowed `SUM(tokens)` check.

Library: adopt `governor` 0.10 + `tower_governor` 0.8 (library-candidates.json G3); reuse existing `redis` dep; do NOT add redis-rate crates.

## Depends on
- (none — first change; extends existing usage_events + MaxTokenBudget)

## Scope
IN: windowed token budgets, per-key/per-team scoping, request-rate limiting, block-on-threshold, Postgres fallback. OUT: cost($)-based budgets (token-based only this change), distributed accuracy as a hard requirement (Redis optional).

## Tasks
- [ ] Add `governor = "0.10"` + `tower_governor = "0.8"` to flint-gate-core Cargo.toml
- [ ] Extend `MaxTokenBudgetConfig` with `window` + `scope` fields (serde defaults preserve lifetime behavior)
- [ ] Implement in-process `tower_governor` request-rate layer on the proxy router with key extractor (API-key/identity)
- [ ] Implement Redis Lua window counters (token budget + rate limit) behind `redis-l2`
- [ ] Wire enforcement into `middleware/pipeline.rs`: block with typed error when over threshold
- [ ] Implement Postgres `SUM(tokens)` windowed fallback when `redis-l2` disabled
- [ ] Unit + integration tests (budget block, rate-limit block, fallback path); ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
