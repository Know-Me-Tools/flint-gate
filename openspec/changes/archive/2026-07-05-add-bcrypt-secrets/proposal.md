# add-bcrypt-secrets

## Why
OAuth client secrets are hashed with **unsalted SHA-256** (`sha256_hex`). That is
defensible for a 256-bit CSPRNG token but offers no work factor against a DB
leak and is unsafe for any operator-chosen secret. (Prior-phase debt / G2)

## What Changes
Adopt **`bcrypt`** (the phase's one deliberate new crate — D-C02) for
`oauth_clients.secret_hash`:
- `create_oauth_client` hashes the CSPRNG secret with `bcrypt::hash` (internal
  salt) and remains the **only** secret-insertion path.
- `verify_client_credentials` **format-sniffs** the stored hash (operator decision):
  a bcrypt hash (`$2b$…`) verifies via `bcrypt::verify`; a legacy SHA-256 row
  verifies the old way and is **transparently re-hashed to bcrypt** on the next
  successful auth. No operator action, no lockout, clean cutover.

## Design
- Add `bcrypt = "0.19"` to `flint-gate-core`.
- The verify lookup changes from `WHERE client_id = $1 AND secret_hash = $2`
  (hash-equality) to **fetch the client row by `client_id`, then KDF-verify the
  presented secret** — a KDF's per-hash salt precludes a hash-equality lookup.
- A `SecretHash` helper: `hash(raw) -> String` (bcrypt), `verify(raw, stored) ->
  bool` (format-sniff bcrypt vs legacy sha256), `needs_rehash(stored) -> bool`.
- On a successful legacy verify, update the row's `secret_hash` to a bcrypt hash.

## Depends on
- `add-oauth-endpoint-hardening` (both touch the client-credentials verify path;
  land the endpoint auth first).

## Scope
IN: bcrypt hashing for new clients, format-sniff verify + transparent re-hash of
legacy rows, CSPRNG-only insertion enforced, tests. OUT: argon2/memory-hard KDF
(rejected — overkill for a 256-bit secret); a data migration script (transparent
re-hash on use handles it).

## Tasks
- [ ] Add `bcrypt = "0.19"`; `SecretHash` helper (bcrypt hash + format-sniff verify + needs_rehash)
- [ ] `create_oauth_client` mints CSPRNG secret → bcrypt hash (only insertion path)
- [ ] `verify_client_credentials`: fetch client by id → verify (bcrypt `$2b$` else legacy sha256) → transparently re-hash legacy to bcrypt on success
- [ ] Tests: bcrypt round-trip verify, wrong secret denied, legacy sha256 row still verifies + gets re-hashed, needs_rehash logic; constant-time via bcrypt
- [ ] Docs: note the KDF + migration behavior; `cargo check/clippy/test --workspace` green
