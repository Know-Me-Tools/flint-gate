# add-client-creds-introspection

## Why
Service-to-service (non-human) agents need a first-class identity path, and
upstreams need to introspect tokens the gateway mints. Neither exists today
(Goal G3 — OAuth2 client-credentials + RFC 7662 introspection).

## What Changes
Hybrid approach (D-B03), **federate-first**:
- **RFC 7662 introspection** `POST /oauth/introspect` for **gateway-minted tokens**
  (reuse `jwt_verify` → `active`, `scope`, `aud`, `exp`, `client_id`). Where Ory
  Hydra is the AS, the gateway can **consume/proxy Hydra's native introspection**
  for Hydra-issued tokens (Hydra owns RFC 7662).
- **`grant_type=client_credentials`** on `POST /oauth/token`: mint a gateway-scoped
  service token from a `client_id`/`client_secret` (reuse `JwtMinter`); client
  store extends the existing `api_keys` table. Where Hydra is the AS, prefer
  Hydra's client-credentials.

## Design
- Client store: extend `api_keys` (or a sibling `oauth_clients` table) with a
  hashed `client_secret` and grant metadata; verify with constant-time compare.
- `client_credentials` grant → verify client → mint a service token (audience,
  scopes, `client_id` claim) via `JwtMinter`.
- `/oauth/introspect` → verify the presented token via `jwt_verify`; return the
  RFC 7662 response shape. Unknown/expired/invalid → `{"active": false}` (never leak).
- Optional `introspection_delegate: { hydra_admin_url? }` seam to forward opaque,
  Hydra-issued tokens to Hydra's introspection endpoint.

## Depends on
- `add-token-exchange` (shares the `/oauth/token` surface + minting patterns).

## Scope
IN: `client_credentials` grant, client store (hashed secret), RFC 7662
`/oauth/introspect` for gateway-minted tokens, Hydra-introspection consume seam,
tests. OUT: full opaque-token store beyond api-keys reuse (bearer-JWT-first);
standing up a parallel authorization server; dynamic client registration.

## Tasks
- [ ] Client store: extend api_keys / add `oauth_clients` with hashed `client_secret` + grant metadata; constant-time verify
- [ ] `grant_type=client_credentials` on `/oauth/token`: verify client → mint service token (client_id + scopes + aud) via JwtMinter
- [ ] `POST /oauth/introspect` (RFC 7662): verify gateway-minted token via jwt_verify → active/scope/aud/exp/client_id; inactive→`{"active":false}`
- [ ] Hydra introspection consume seam (`introspection_delegate.hydra_admin_url?`) — defined + wired behind config, off by default
- [ ] Tests: client-creds mint+verify round-trip, bad secret denied (constant-time), introspection active/inactive, `degrades_to_deny`/`active:false` on malformed
- [ ] Docs: endpoints + client config; `cargo check/clippy/test --workspace` green
