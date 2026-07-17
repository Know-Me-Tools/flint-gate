# Analysis — agent-gateway-hardening-and-exposure

_Generated: 2026-07-04 · Mode: **stack-specified** (rust-axum-tokio-postgres, unchanged)_
_Research: Tier 3 `cargo search` (KDF crates), Tier 4 web (RFC 7662 §2.1). Within budget._

## Governing constraints (carried)

Federate any JWKS-capable IdM (Ory reference); never an IdP; single-binary, no
sidecar; jsonwebtoken@9 pin; **fail-closed with a `degrades_to_deny` test on
every new auth path**; **audit every authorize entry point** (last phase's miss).

## Build-vs-adopt calls (per goal)

### G1 · OAuth endpoint auth + rate limiting → **REUSE existing client store + governor** (hand-wire)
- **Decision:**
  - **Auth:** authenticate `/oauth/introspect` (and optionally `/oauth/token`)
    with **OAuth client credentials** — `client_id` + `client_secret` (POST params
    or HTTP Basic), verified against the **existing `oauth_clients` store**
    (`verify_client_credentials`, already built for the client-credentials grant).
    This is exactly what RFC 7662 §2.1 mandates and reuses code we already have.
  - **Rate-limit:** apply the existing **`build_governor_layer`** as a per-endpoint
    tower layer on the OAuth sub-router (independent of the default-off global
    governor), plus a failed-attempt path that returns `invalid_client` uniformly.
    Gate the Hydra-delegate path behind introspection auth.
- **Why:** RFC 7662 **MANDATES** introspection-endpoint auth to stop token
  scanning ([RFC 7662 §2.1](https://datatracker.ietf.org/doc/html/rfc7662#section-2.1));
  client-credentials is the standard mechanism and flint-gate already verifies
  them. The governor layer already exists and is composable. **Zero new crates for G1.**
- **Rejected:** a bespoke admin credential (off-standard); leaving it
  network-restriction-only (RFC says MUST authenticate).
- **Open → Spec:** does `/oauth/token` client-credentials grant *also* require the
  client to authenticate to reach the endpoint (it already presents client creds
  in-body), or is per-endpoint rate-limit enough there? (lean: rate-limit +
  in-body client-auth is sufficient for `/oauth/token`; hard client-auth gate on
  `/oauth/introspect`.)
- **Confidence:** HIGH.

### G2 · Slow salted KDF for client secrets → **ADOPT `bcrypt`** (the phase's one new crate)
- **Decision:** replace unsalted SHA-256 (`sha256_hex`) for `oauth_clients.secret_hash`
  with **`bcrypt`** (`0.19`, pure-Rust, batteries-included: internal salt,
  `hash`/`verify` API). Verify becomes **fetch-client-by-id → `bcrypt::verify`**
  (a KDF can't do a `WHERE secret_hash = $2` lookup). Enforce the CSPRNG
  `create_oauth_client` as the only secret-insertion path.
- **Why:** the client secret is already a 256-bit CSPRNG token, so bcrypt's work
  factor is comfortably sufficient; `bcrypt@0.19.2` is **stable**, minimal-API,
  and pure-Rust (single-binary-safe). `argon2` is theoretically stronger but its
  crate is mid-release-churn (latest is `0.6.0-rc`; RustCrypto stable line lags),
  and Argon2id's memory-hardness is overkill for a high-entropy machine secret.
  KISS wins.
- **Rejected:** `argon2` (rc-version churn; memory-hardness unnecessary here),
  `scrypt` (same overkill), keeping SHA-256 (no work factor).
- **Note:** this **breaks the prior two-phase all-reuse streak** — a deliberate,
  security-justified single new crate. `rand@0.8` (salt entropy) is already present.
- **Migration:** existing `oauth_clients` rows carry SHA-256 hashes; new rows use
  bcrypt. Detect the format on verify (bcrypt hashes start `$2b$`) or, since this
  is pre-GA, document a re-issue. (lean: format-sniff verify for a clean cutover.)
- **Confidence:** HIGH.

### G3 · Identity-classification edges → **EXTEND existing code** (3 small fixes)
- **G3a API-key → Service:** in `auth/api_key.rs::build_result`, set
  `Identity.kind = Service` (an API key is a non-human service credential), so
  `Service::` policies apply and the client is covered by the NHI revocation list.
- **G3c Kratos self-classification:** in `identity.rs::derived_kind`, skip the
  `act` fallback when the identity carries a `session_id` (the Kratos marker) —
  `flint_kind` is already gateway-signed-only and safe. Kratos `metadata_public`
  never feeds a kind promotion.
- **G3d transactional NHI audit:** wrap the status flip + audit-row insert in one
  `sqlx` transaction (mirror the `.begin()` pattern at `db/mod.rs:834`), so a
  revoke is audited-before-effect.
- **Why:** each is a targeted correctness/robustness fix on existing code; no new deps.
- **Confidence:** HIGH.

### G4 · Chained delegation + Hydra-delegate → **HAND-ROLL (reject-first for actor_token) + wire the Hydra seam**
- **Decision:**
  - **`actor_token`:** this phase, **verify-or-reject** — if an `actor_token` is
    present, verify it against the same JWKS verifier and record a proper `act`
    chain; if verification is out of scope for the change, **reject present-but-
    unsupported (fail-closed)** rather than silently ignore it (today it's parsed
    and dropped). Full multi-hop chaining can be a follow-up.
  - **`delegate_to_hydra`:** wire the seam — when set, proxy the exchange to the
    configured Hydra token endpoint (reuse the `reqwest` client), with the known
    external-`aud` caveat ([ory/hydra#3723](https://github.com/ory/hydra/issues/3723)).
- **Why:** silently ignoring a security-relevant request parameter is the exact
  fail-open pattern this phase exists to kill; reject-or-verify closes it. The
  Hydra proxy completes the federate-first delegation story.
- **Rejected:** silently ignoring `actor_token` (fail-open); a new delegation crate.
- **Open → Spec:** verify-and-chain now vs reject-if-present this phase (lean:
  reject-if-present fail-closed, defer full chaining).
- **Confidence:** MEDIUM.

## Recurring principles (carry to Spec)

- **RFC-mandated auth is not optional** — G1's introspection auth is a MUST, not a nice-to-have.
- **Fail-closed on every new/hardened path** + `degrades_to_deny` tests.
- **Audit every authorize entry point** — re-check both the per-tool and route gates when touched.
- **One deliberate new crate (`bcrypt`), security-justified** — otherwise reuse.

## Open questions for Spec

1. `/oauth/token` hard client-auth gate vs rate-limit-only (lean: rate-limit + in-body client-auth).
2. bcrypt migration: format-sniff verify vs re-issue (lean: format-sniff for clean cutover).
3. G4 `actor_token`: verify-and-chain now vs reject-if-present (lean: reject-if-present).
4. Rate-limit backend: in-process governor per-endpoint (this phase) vs Redis window
   counters for cross-replica accuracy (carried Redis-L2 question; lean: governor now).

## Sources
- [RFC 7662 §2.1 — introspection endpoint MUST require authentication](https://datatracker.ietf.org/doc/html/rfc7662#section-2.1)
- [RFC 6749 — OAuth 2.0 (client authentication)](https://datatracker.ietf.org/doc/html/rfc6749)
- [ory/hydra#3723 — external-token `aud` handling on exchange](https://github.com/ory/hydra/issues/3723)
