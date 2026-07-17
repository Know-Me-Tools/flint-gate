# Analysis — agent-identity-and-delegation

_Generated: 2026-07-04 · Mode: **stack-specified** (rust-axum-tokio-postgres, fixed last phase)_
_Research: Tier 1 `gh search code`, Tier 3 `cargo search`, Tier 4 web (Ory docs/issues). Budget: within caps._

## Governing constraint (user directive)

> **Support any identity-management system that has a pathway to generate JWTs we
> can consume. Ory Kratos / Hydra are our standard.**

This is architecturally decisive and shapes every call below: **flint-gate stays a
resource-server / policy gateway that _federates_ — it is not, and must not become,
an IdP/authorization-server.** Ory is the reference integration, but no design may
hard-depend on Ory-specific behavior; the contract is "a verifiable JWT (JWKS +
iss/aud) in, an authorization decision / delegated token out."

## Landscape findings (evidence)

### RFC 8693 token-exchange — no Rust _server_ crate; universally hand-rolled
- Tier 1 `gh search code`: every real RFC 8693 usage in Rust (`awslabs/aws-sdk-rust`
  ssooidc, `openai/codex` login server, `warpdotdev/warp`) is a **hand-rolled
  `application/x-www-form-urlencoded` POST** with `grant_type=urn:ietf:params:oauth:
  grant-type:token-exchange`, `subject_token`, `subject_token_type`. No crate
  abstracts the RS/exchange _server_ side.
- `oauth2` crate (5.0) and `openidconnect` (4.0) are **client-shaped** (last phase
  D-A03 rejected them for the same reason); they don't provide a server exchange
  endpoint, and `oauth2@5`/`openidconnect@4` also pull a newer TLS/http stack.

### Ory Hydra — owns RFC 7662 natively; RFC 8693 is real-but-caveated
- **RFC 7662 introspection: first-class in Hydra** (dedicated admin introspection
  endpoint). ([Ory introspection docs](https://www.ory.com/docs/hydra/guides/oauth2-token-introspection))
- **RFC 8693 token-exchange: Hydra v2 release notes claim "fully supported," but
  users report real breakage** — notably `aud`-claim handling when exchanging an
  _external_ OIDC token ([ory/hydra#3723](https://github.com/ory/hydra/issues/3723),
  [discussion #3359](https://github.com/ory/hydra/discussions/3359)). So Hydra
  can own the exchange where deployed and configured, but flint-gate **cannot
  assume** every federated IdM implements 8693.

### Version constraint holds
- `jsonwebtoken` latest is **10.4**, but the workspace is **pinned to 9** (D-A03).
  All candidates must work on 9; `biscuit` (JOSE, 0.8) is an alternative but would
  duplicate the JWT stack — rejected.

### Admin-plane auth — reuse, don't add a framework
- `axum-login@0.18` / `tower-sessions@0.15` exist and are healthy, but the codebase
  already has JWT + Kratos verification (`auth/jwt_verify.rs`, `auth/kratos.rs`).
  Adding a session framework would duplicate identity handling and pull a session
  store. A thin `axum::middleware::from_fn` layer that reuses an existing
  `AuthProviderConfig` is the smaller, on-identity choice.

## Build-vs-adopt calls (per goal)

### G1 · Admin API authentication → **REUSE existing auth providers** (hand-wire middleware)
- **Decision:** add an admin-auth **tower middleware** (`from_fn`) that verifies the
  request against a configured `AuthProviderConfig` — **reuse `JwtAuthConfig` or
  `KratosAuthConfig`** (the Ory-standard path). New `admin_auth` config block;
  loopback-dev keeps an explicit `none` opt-in.
- **Why:** verification primitives already exist and are tested; no new crate, no
  new identity model, single-binary preserved. Directly satisfies "Ory is standard"
  (Kratos session or Hydra-issued JWT both work) **and** "any IdM with a JWT" (any
  JWKS-backed JWT provider).
- **Rejected:** `axum-login`/`tower-sessions` (duplicate identity + session store),
  a bespoke admin password (weaker, off-standard).
- **Confidence:** HIGH.

### G2 · RFC 8693 token-exchange → **FEDERATE-FIRST + gateway-local fallback** (hand-roll surface, reuse `JwtMinter`)
- **Decision:** two-mode exchange:
  1. **Delegate mode (preferred where configured):** proxy the exchange to the
     configured Ory Hydra token endpoint (Hydra owns RFC 8693). flint-gate verifies
     the result and binds audience (RFC 8707, reuse existing enforcement).
  2. **Gateway-local mode (fallback for IdMs without 8693):** hand-roll the
     `grant_type=token-exchange` endpoint — verify `subject_token` via
     `auth/jwt_verify.rs`, apply scope **downscoping**, and mint a delegated token
     with an `act` claim via **`JwtMinter.mint()`**. This is the "any IdM that can
     produce a JWT" guarantee: as long as the incoming token verifies against a
     JWKS, flint-gate can issue a downscoped delegated token.
- **Why:** no Rust server crate exists (hand-roll confirmed idiomatic); the
  issuance/verification halves already exist; federate-first avoids re-implementing
  an AS while the fallback honors the vendor-neutral directive. Mirrors last phase's
  successful "hand-roll the surface, reuse jsonwebtoken" pattern (D-A03).
- **Rejected:** `oauth2`/`openidconnect` (client-only), becoming a full AS.
- **Open question → Spec:** is delegate-mode in scope now, or fallback-only first
  with a Hydra-delegate follow-up? (lean: ship gateway-local first — it's the
  vendor-neutral core — with a config seam for Hydra-delegate.)
- **Confidence:** HIGH (fallback), MEDIUM (delegate-mode Hydra `aud` quirks).

### G3 · Client-credentials + RFC 7662 introspection → **HYBRID: consume Hydra + hand-roll local**
- **Decision:**
  - **Introspection:** flint-gate exposes an RFC 7662 `/introspect` for
    **gateway-minted tokens** (reuse `jwt_verify` → active/scope/aud/exp), and can
    **proxy/consume Hydra's introspection** for Hydra-issued opaque tokens (Hydra
    owns 7662 natively).
  - **Client-credentials:** hand-roll the `grant_type=client_credentials` grant for
    gateway-scoped service tokens (mint via `JwtMinter`, client store extends the
    api-keys table). Where Hydra is the AS, **prefer Hydra's client-credentials**.
- **Why:** Hydra already implements both; duplicating them wholesale would be
  off-identity. flint-gate adds only the local surface for its own minted tokens +
  a federation seam.
- **Rejected:** standing up a parallel full OAuth2 AS.
- **Confidence:** HIGH.

### G4 · NHI / workload identity as Cedar principals → **EXTEND engine (thread principal type) + lifecycle store**
- **Decision:** (a) thread a principal **entity-type** (not just id) from `Identity`
  through `authz/engine.rs::authorize` — `make_uid(type_name, id)` is **already
  generic**, so add `Agent`/`Service` alongside `User`; (b) add an identity-**kind**
  to `Identity` (derived from the auth provider / token claims); (c) a lifecycle
  store (issue/rotate/revoke) surfaced via Admin API + UI + audit, with
  revocation-takes-effect on the next authorize.
- **Why:** the Cedar plumbing is close; the real work is the lifecycle store +
  revocation semantics. Keeps NHI a first-class policy principal without a new
  policy engine.
- **Open question → Spec:** does an agent identity map to a distinct Cedar entity
  **type** (`Agent::`) or a `User` with a `kind` attribute? (lean: distinct type —
  lets policies say "agents may X, users may not" cleanly.)
- **Confidence:** MEDIUM.

## Recurring principles (carry to Spec)

- **Federate, don't become an IdP.** Every grant/introspection has a "prefer the
  configured AS (Hydra)" seam plus a gateway-local implementation for vendor-neutrality.
- **JWT-in contract is the vendor-neutral boundary** — anything that verifies against
  a configured JWKS (iss/aud) is a valid identity source. Ory is the tested default.
- **Fail-closed + `degrades_to_deny` test on every new auth/exchange path** (prior-phase lesson).
- **jsonwebtoken@9 pin, single-binary, no sidecar** — unchanged.

## Open questions for Spec

1. G2 delegate-mode (proxy to Hydra) in-scope now vs gateway-local-first? (lean: local-first)
2. G4 agent principal: distinct Cedar `Agent::` type vs `User` + `kind` attribute? (lean: distinct type)
3. G3 opaque-token store: reuse api-keys table vs new token store? Bearer-JWT-only first?
4. Admin authn default posture: `none`-on-loopback opt-in vs always-require when `admin_auth` set?
5. Carried: Redis-L2 hard-dep for cross-replica budgets (blocks multi-replica identity scaling).

## Sources
- [RFC 8693 — OAuth 2.0 Token Exchange](https://datatracker.ietf.org/doc/html/rfc8693)
- [RFC 7662 — OAuth 2.0 Token Introspection](https://datatracker.ietf.org/doc/html/rfc7662)
- [Ory Hydra — token introspection guide](https://www.ory.com/docs/hydra/guides/oauth2-token-introspection)
- [ory/hydra#3723 — external OIDC `aud` handling on exchange](https://github.com/ory/hydra/issues/3723)
- [ory/hydra discussion #3359 — RFC 8693 support status](https://github.com/ory/hydra/discussions/3359)
