# Assessment — agent-gateway-hardening-and-exposure

_Generated: 2026-07-04 · Phase status: assess_pending · Changes: 0/0 (not yet planned)_

Assessed the flint-gate codebase against the four hardening goals. This phase is
**targeted remediation of last phase's recorded debt** — mostly small, surgical
changes to code that already exists, plus one genuinely new surface (endpoint
auth/rate-limiting) and one deferred feature (chained delegation).

## Codebase baseline (what exists to harden)

| Surface | Current state | Reuse for this phase |
|---------|--------------|----------------------|
| `main.rs` OAuth router | `/oauth/token` + `/oauth/introspect` merged onto the proxy app with **no auth layer, no rate-limit** | G1 wraps it with auth + a rate-limit layer |
| `ratelimit/governor_layer.rs` | `build_governor_layer(per_second, burst)` — reusable tower layer, currently applied only to the proxy router | G1 reuses it per-endpoint |
| `db/mod.rs` `sha256_hex` + `oauth_clients.secret_hash` | **unsalted SHA-256** | G2 swaps to a slow salted KDF |
| `auth/api_key.rs` `build_result` | `Identity { id, ..Default }` → `kind = User`, no `metadata_public` | G3a sets Service kind |
| `admin/mod.rs` `audit_nhi_event` | called **after** the DB mutation returns (separate await; best-effort) | G3d makes it transactional (a `.begin()` txn pattern already exists at `db/mod.rs:834`) |
| `auth/identity.rs` `derived_kind` | `flint_kind` (gateway-signed) is safe; the `act` fallback also fires for a Kratos identity (`kind = User`, has `session_id`) | G3c gates the `act` fallback off the Kratos path |
| `auth/token_exchange.rs` | `actor_token` is a **parsed-but-unused** field; `delegate_to_hydra` is comment-only | G4 verifies `actor_token` + wires the Hydra-delegate seam |

## Gap analysis (per goal)

### G1 · OAuth endpoint auth + rate limiting — **NOT MET** (CRITICAL, build first)

- **Evidence:** in `main.rs` the OAuth router is `Router::new().route("/oauth/token", …)
  .route("/oauth/introspect", …).with_state(oauth_state)` merged into `proxy_app`
  with **no `.layer()`** — no authentication, no rate limiter. The global governor
  (`server.rate_limit`) defaults **off** and is coarse/per-replica anyway.
- **Gap:** `/oauth/token` is an unthrottled client-credentials guessing surface;
  `/oauth/introspect` is an unauthenticated token oracle that, with the Hydra
  delegate configured, proxies to Hydra's admin API. These are the only
  unauthenticated security-sensitive surfaces blocking safe exposure.
- **Shape of fix:** per-endpoint auth (client auth on the token endpoint per RFC
  6749; a dedicated introspection credential per RFC 7662) + a per-endpoint
  `build_governor_layer` (or a failed-attempt backoff), applied as a tower layer
  to the OAuth sub-router. Gate the Hydra-delegate path so it is unreachable
  unauthenticated.
- **Feasibility:** HIGH — the rate-limit layer + the client-credentials verifier
  both already exist; this composes them onto the OAuth router.
- **Open question:** endpoint-auth mechanism (client-creds vs dedicated
  credential vs network-restriction) — decide in Analyze.

### G2 · Slow salted KDF for client secrets — **NOT MET** (HIGH)

- **Evidence:** `sha256_hex` (unsalted SHA-256) hashes the client secret; no
  argon2/bcrypt/scrypt crate is in `Cargo.toml`.
- **Gap:** SHA-256 is fast; defensible for a 256-bit CSPRNG token but unsafe for
  anything operator-chosen, and offers no work-factor against a DB leak.
- **Shape of fix:** adopt `argon2` (or `bcrypt`) for `oauth_clients.secret_hash`;
  migrate the verify path to the KDF's constant-time verify; **enforce that the
  CSPRNG `create_oauth_client` path is the only secret-insertion route.**
- **Feasibility:** HIGH — a single crate + swap two functions (`create`/`verify`).
  Note: KDF verify is per-secret (can't do a `WHERE secret_hash = $2` lookup), so
  verify becomes "fetch client by id, then KDF-verify the secret" — a small refactor.
- **Feasibility caveat:** this is the **first new crate** of the phase (breaks the
  prior all-reuse streak) — but a security-justified one.

### G3 · Identity-classification edges — **PARTIAL / NOT MET** (MEDIUM, cross-cutting)

- **G3a API-key → Service (NOT MET):** `build_result` leaves `kind = User`, so an
  API-key workload authorizes as `User::"<client_id>"` and escapes the NHI
  revocation list. Fix: set `kind = Service` (or stamp `flint_kind`) in `build_result`.
- **G3c Kratos self-classification (PARTIAL):** `flint_kind` is gateway-signed-only
  (Kratos can't set it — safe), but the `act` fallback in `derived_kind` still fires
  for a Kratos identity. Fix: skip the `act` fallback when the identity carries a
  `session_id` (Kratos marker), or don't feed Kratos `metadata_public` into derivation.
- **G3d NHI audit transactionality (NOT MET):** `audit_nhi_event` runs after the
  status flip as a separate best-effort await. Fix: write the audit row in the same
  transaction as the status change (mirror the `.begin()` pattern at `db/mod.rs:834`).
- **Feasibility:** HIGH (each is a small, local change).

### G4 · Chained delegation + Hydra-delegate exchange — **NOT STARTED** (MEDIUM)

- **Evidence:** `actor_token` is parsed into `TokenExchangeRequest` but never
  verified or used; `delegate_to_hydra`/`hydra_token_url` config exists but the
  exchange always runs locally.
- **Gap:** RFC 8693 delegation beyond a single `act` isn't verified (an
  `actor_token` is silently ignored — should be verified or explicitly rejected);
  the Hydra-delegate token-exchange mode is unbuilt.
- **Shape of fix:** verify `actor_token` (reuse the subject-token verifier) and
  record a proper `act` chain, OR reject when present-but-unsupported (fail-closed);
  wire `delegate_to_hydra` to proxy the exchange to the configured Hydra token endpoint.
- **Feasibility:** MEDIUM — verification reuses existing primitives; Hydra proxy is
  a new reqwest call with the known external-`aud` caveat (ory/hydra#3723).

## Dependency & build order (recommended)

```
G1 endpoint auth + rate-limit ─→ the exposure gate; nothing else exposes safely first
G2 slow-KDF secrets ──────────→ independent; pairs with G1's guessing-surface fix
G3 classification edges ──────→ independent small fixes (a/c/d)
G4 chained delegation ────────→ depends on G1/G2 landing; completes delegation
```

## Cross-cutting constraints (carry into Spec)

- **Fail-closed everywhere** — every new auth path needs a `degrades_to_deny` test
  (the two-phase recurring lesson; last phase caught 5 HIGH fail-open defects).
- **Audit every authorize entry point, not just the engine** — last phase's
  route-level-shim miss. Any new auth surface must be checked at *every* call site.
- **Single-binary, no sidecar; jsonwebtoken@9 pin.** G2 adds the first new crate
  (argon2/bcrypt) — security-justified; keep it single-binary-friendly (pure Rust).
- **Federate, don't become an IdP** — G4's Hydra-delegate prefers the AS, local is fallback.

## Open questions for Analyze

1. **G1 endpoint-auth mechanism:** client-credentials client-auth on `/oauth/token`
   + a dedicated introspection credential on `/oauth/introspect`, vs
   network-restriction-only, vs both? (leans: real client-auth + rate-limit.)
2. **G1 rate-limit backend:** reuse the in-process governor per-endpoint, or the
   Redis window counters for cross-replica accuracy? (carried Redis-L2-hard-dep question.)
3. **G2 KDF choice:** `argon2` vs `bcrypt` (both pure-Rust); Argon2id is the modern default.
4. **G4 actor_token:** verify-and-chain now, or reject-if-present this phase and
   defer full chaining? (leans: reject-if-present fail-closed first, chain later.)
