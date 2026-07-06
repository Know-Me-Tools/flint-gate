# add-oauth-endpoint-hardening

## Why
`POST /oauth/token` and `POST /oauth/introspect` (added last phase) are
**unauthenticated and unrate-limited** on the proxy port. RFC 7662 §2.1 makes
introspection-endpoint authentication a **MUST** (token-scanning defense), and
the client-credentials guessing surface + the Hydra-delegate oracle both block
safe exposure. This is the phase's CRITICAL exposure gate. (Prior-phase debt #1)

## What Changes
Harden the OAuth sub-router (D-C01, all reuse — **zero new crates**):
- **Client authentication on `/oauth/introspect`** — require OAuth client
  credentials (`client_id`+`client_secret`, POST params or HTTP Basic), verified
  against the **existing `oauth_clients` store** (`verify_client_credentials`).
  401 `invalid_client` without valid creds. The **Hydra introspection-delegate**
  path is therefore only reachable authenticated.
- **Per-endpoint rate limiting** on both `/oauth/token` and `/oauth/introspect`
  via the **existing `build_governor_layer`**, independent of the default-off
  global governor. `/oauth/token` keeps its in-body grant credentials (the grant
  already authenticates the caller) plus the rate-limit.

## Design
- New `oauth.rate_limit` config (per_second/burst) + `oauth.introspect_auth`
  toggle (default: required when `introspection_enabled`).
- `introspect_endpoint` extracts client creds (Basic or form `client_id`/
  `client_secret`), calls `verify_client_credentials`; on failure returns 401
  `invalid_client` before any token verification or Hydra delegation.
- The OAuth sub-router gets a `build_governor_layer(...)` tower layer keyed on
  client IP (peer) so a single caller cannot flood either endpoint.

## Depends on
- Reuses `oauth_clients` store + `build_governor_layer`. **Built first** (exposure gate).

## Scope
IN: introspect client-auth (Basic + form), per-endpoint rate-limit on both OAuth
endpoints, Hydra-delegate gated behind introspect auth, config, tests.
OUT: hard client-auth gate on `/oauth/token` beyond its in-body grant creds
(operator decision — rate-limit only there); cross-replica rate limiting (governor
is per-replica; Redis window counters deferred).

## Tasks
- [ ] Add `oauth.rate_limit` (per_second/burst) + `oauth.introspect_auth` config
- [ ] `/oauth/introspect`: extract client creds (HTTP Basic + form) → verify_client_credentials → 401 invalid_client on failure, BEFORE local verify / Hydra delegate
- [ ] Apply per-endpoint `build_governor_layer` to the OAuth sub-router (both endpoints)
- [ ] Tests: introspect without creds → 401; bad creds → 401; valid creds → introspects; Hydra-delegate unreachable unauthenticated; rate-limit trips (fail-closed `degrades_to_deny` on missing/invalid client)
- [ ] Docs: config.example.yaml oauth.rate_limit/introspect_auth + README RFC 7662 auth note; `cargo check/clippy/test --workspace` green
