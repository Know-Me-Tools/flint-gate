# implement-jwt-key-rotation

## Summary
Make the `jwt_signing_keys` table functional with a full key rotation lifecycle: DB CRUD methods, JwtMinter preference for DB-sourced active key, admin endpoints, and cache invalidation on rotation.

## Motivation
The `jwt_signing_keys` table is created at startup (`src/db/mod.rs:54-61`) but never read or written by any code. JWT signing currently flows exclusively through config-file values (`JwtConfig.signing_key_path` / `signing_key_secret` → `main.rs:81-82,170` → `jwt_mint.rs`). The README marks the table as "future" (`README.md:656`). Production deployments need key rotation without process restart.

## Design
1. **Database methods** in `src/db/mod.rs`:
   - `get_active_signing_key() -> Option<JwtSigningKey>` — `SELECT * FROM jwt_signing_keys WHERE active = true ORDER BY created_at DESC LIMIT 1`
   - `list_signing_keys() -> Vec<JwtSigningKey>`
   - `insert_signing_key(id, algorithm, public_key, private_key) -> ()` — sets `active=true`, deactivates others in a transaction
   - `deactivate_signing_key(id) -> ()`

2. **JwtMinter integration** in `src/auth/jwt_mint.rs`:
   - When `Database` is configured and `get_active_signing_key()` returns a key, prefer it over `JwtConfig` file/secret values.
   - Fall back to config when DB is unconfigured or table is empty.

3. **Admin endpoints** in `src/admin/mod.rs` under `/signing-keys`:
   - `GET /signing-keys` — list all keys (never return `private_key`)
   - `POST /signing-keys` — insert new key (deactivates previous active)
   - `DELETE /signing-keys/:id` — deactivate key
   - Mirror the `/api-keys` pattern at `admin/mod.rs:219-318`.

4. **Cache invalidation**: route mutations already send `pg_notify('flintgate_config_changed', 'routes')`. Extend to send `'signing_keys'` and have the cache invalidation listener reload the active key in the minter.

## Tasks
- [ ] Add `JwtSigningKey` struct and `Database` CRUD methods for signing keys
- [ ] Integrate DB-sourced key into `JwtMinter` (prefer DB, fallback to config)
- [ ] Add `/signing-keys` admin endpoints (GET list, POST create, DELETE deactivate)
- [ ] Extend `pg_notify` to fire on signing-key mutations
- [ ] Extend cache invalidation listener to reload minter on `signing_keys` NOTIFY
- [ ] Add tests: insert+activate deactivates previous, minter prefers DB key, fallback to config
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
