# add-token-exchange

## Why
Agents need to act **on behalf of** a user with a **downscoped** token, never the
user's raw credential (confused-deputy prevention, already half-built via the
no-token-passthrough guard). No token-exchange exists today. This is the phase's
strategic core (Goal G2, RFC 8693).

## What Changes
Add a hand-rolled **RFC 8693 token-exchange endpoint** in *gateway-local* mode
(D-B02, operator-confirmed scope): verify the incoming `subject_token` against any
configured JWKS, apply scope **downscoping**, and mint a delegated token carrying
an `act` claim via the existing **`JwtMinter`**. This is the vendor-neutral
guarantee — **any IdM that can produce a verifiable JWT** is a valid subject-token
source; Ory is the reference default.

A `delegate_to_hydra` **config seam** is defined (default off) for a future change
that proxies the exchange to a configured Ory Hydra token endpoint — **not built
in this change** (avoids Hydra's external-`aud` quirks, ory/hydra#3723, for now).

## Design
- `POST /oauth/token` with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`,
  form-encoded `subject_token`, `subject_token_type`, optional `scope`, `resource`,
  `audience`, `actor_token`.
- Verify `subject_token` via `auth/jwt_verify.rs` (JWKS, iss/aud, leeway).
- Downscope: the requested `scope` MUST be a subset of the subject token's scopes;
  reject (fail-closed) otherwise.
- Mint the delegated token via `JwtMinter.mint(identity, additional_claims)` with
  an `act` claim (`{"act": {"sub": "<agent principal>"}}`) and RFC 8707 audience
  binding (reuse existing enforcement).
- Config: `token_exchange: { enabled, delegate_to_hydra: false, hydra_token_url? }`
  — only the local path is wired; the Hydra fields are the documented seam.

## Depends on
- `add-admin-authn` (config/provider patterns land first). Reuses `jwt_mint`,
  `jwt_verify`, `mcp_metadata` audience enforcement.

## Scope
IN: gateway-local RFC 8693 exchange endpoint, subject-token verification, scope
downscoping, `act`-claim delegated-token minting, audience binding, config seam
for Hydra-delegate (defined, not built), tests.
OUT: Hydra-delegate proxy mode (future change); RFC 8693 `actor_token` chained
delegation beyond a single `act` (future); becoming a full OAuth2 AS.

## Tasks
- [ ] Add `token_exchange` config (enabled + `delegate_to_hydra:false` seam + `hydra_token_url?`)
- [ ] `POST /oauth/token` token-exchange handler: parse RFC 8693 form params, verify `subject_token` via jwt_verify (JWKS)
- [ ] Scope downscoping: requested scope ⊆ subject scopes else fail-closed 400 `invalid_scope`
- [ ] Mint delegated token via JwtMinter with `act` claim + RFC 8707 audience binding
- [ ] Tests: valid downscoped exchange, scope-escalation denied, invalid/expired subject_token denied, `degrades_to_deny` on malformed input; unknown-IdM JWT (any JWKS) accepted
- [ ] Docs: config seam + endpoint; `cargo check/clippy/test --workspace` green
