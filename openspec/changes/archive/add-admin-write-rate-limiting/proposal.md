# add-admin-write-rate-limiting

## Goal

G1: Wire the existing `tower_governor` / `build_governor_layer` rate-limit
infrastructure to the admin router's protected sub-router so all admin write
endpoints (`POST`, `PUT`, `DELETE`) return `429 Too Many Requests` when a
per-credential burst is exceeded. No new dependencies — `tower_governor` and
`CredentialKeyExtractor` are already in `Cargo.toml` and `src/ratelimit/`.

## Scope

- `crates/flint-gate-core/src/config/types.rs` — add `admin_rate_limit:
  Option<RateLimitConfig>` field to `ServerConfig`.
- `crates/flint-gate-core/src/admin/mod.rs` — `admin_router_with_auth`
  accepts an `Option<GovernorLayer<...>>` and layers it on the protected
  sub-router.
- `crates/flint-gate/src/main.rs` — build the layer from
  `initial_config.server.admin_rate_limit` and pass it in.
- `config.example.yaml` — document `server.admin_rate_limit` block.
- Tests: protected route returns 429 after burst; `/health` unaffected.

## Security requirements

- Rate-limit key: `CredentialKeyExtractor` (credential hash → IP fallback) —
  same extractor used on the proxy router, prevents per-IP bypass via
  credential rotation and per-credential bypass via IP rotation.
- Default: `None` (disabled on loopback-dev); explicitly opt-in for production.
- The limiter is per-replica (in-process governor); cross-replica fairness is
  out of scope (same documented constraint as `server.rate_limit`).
- `require_shared_backend` field in `RateLimitConfig` does NOT apply to admin
  (only to `oauth.rate_limit`); this must be documented in the config comment.
