# add-actor-token-and-hydra-delegate

## Why
Two delegation edges left open last phase: the RFC 8693 `actor_token` request
parameter is **parsed but silently ignored** (a security-relevant param dropped
— the exact fail-open pattern this phase exists to kill), and the
`delegate_to_hydra` token-exchange seam is defined-but-unbuilt. (G4)

## What Changes
(D-C04):
- **`actor_token`: reject-if-present (fail-closed)** — operator decision. When a
  token-exchange request carries an `actor_token`, **reject it** with a clear
  `invalid_request` / unsupported error rather than silently ignoring it. Closes
  the silent-drop now; full verify-and-chain multi-hop delegation is a clean
  follow-up (out of scope this change).
- **Wire the Hydra-delegate exchange:** when `token_exchange.delegate_to_hydra`
  is set (with `hydra_token_url`), proxy the exchange to the configured Ory Hydra
  token endpoint (reuse the `reqwest` client) instead of minting locally — the
  federate-first path (Hydra owns RFC 8693). Local minting stays the default;
  the known external-`aud` caveat (ory/hydra#3723) is documented.

## Design
- `token_exchange` handler: if `req.actor_token.is_some()` → return
  `ExchangeError::UnsupportedActorToken` → 400 `invalid_request` (fail-closed),
  before any minting.
- Hydra-delegate: when enabled, POST the RFC 8693 form to `hydra_token_url`,
  return Hydra's token response; on transport/non-2xx error, fail closed (deny)
  rather than falling back to local mint (avoids a confused mode).

## Depends on
- `add-oauth-endpoint-hardening` + `add-bcrypt-secrets` (the OAuth surface is
  hardened first). Built last.

## Scope
IN: reject `actor_token` fail-closed, Hydra-delegate token-exchange proxy behind
config, tests. OUT: full multi-hop `act` chaining / `may_act` verification
(follow-up); Hydra external-`aud` remediation (documented caveat).

## Tasks
- [ ] `actor_token` present → reject (400 invalid_request / unsupported), before minting — no silent-ignore
- [ ] Wire `delegate_to_hydra`: when set, proxy the exchange to `hydra_token_url` (reqwest); fail closed on transport/non-2xx
- [ ] Tests: actor_token present → rejected (fail-closed); absent → normal act-claim exchange; delegate-mode forwards to Hydra (wiremock) + fails closed on Hydra error
- [ ] Docs: config seam + external-aud caveat; `cargo check/clippy/test --workspace` green
