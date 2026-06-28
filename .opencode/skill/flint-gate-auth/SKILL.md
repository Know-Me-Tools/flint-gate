---
name: flint-gate-auth
description: Configure flint-gate auth providers (Kratos session, JWT/JWKS, API keys, anonymous) and rotate outbound JWT signing keys. Use when the user says "setup auth flint gate" or "rotate jwt key".
version: 0.1.0
license: MIT
---

# flint-gate-auth

Two distinct concerns: **inbound auth providers** (validate caller identity) and **outbound JWT minting** (sign tokens for upstreams via the `claims_enhancement` hook).

## Inbound auth providers

Defined under `auth_providers:` in `config.yaml`. Each entry is `name → { type, ... }`.

### Kratos session

Validates an Ory Kratos session by calling `GET /sessions/whoami` on `base_url`. Forwards the configured session cookie.

```yaml
auth_providers:
  kratos_session:
    type: kratos
    base_url: "http://kratos:4433"
    forward_cookies: true
    session_cookie: "ory_kratos_session"
```

Reference: `auth: kratos_session` on a route or site.

### JWT (JWKS)

Verifies `Authorization: Bearer <token>`. Fetches keys from `jwks_url`, validates `iss` and `aud` with clock `leeway_seconds`.

```yaml
auth_providers:
  bearer_jwt:
    type: jwt
    jwks_url: "https://auth.example.com/.well-known/jwks.json"
    issuer:   "https://auth.example.com"
    audience: "flint-gate"
    leeway_seconds: 5
```

The JWKS document is cached; key rotation at the issuer propagates on next fetch. To force-refresh, restart or hot-reload the config.

### API key (Phase 3)

Extracts the key from a header, SHA-256 hashes it, and looks up the hash in the `api_keys` table. Plaintext is never stored.

```yaml
auth_providers:
  api_key:
    type: api_key
    header: "X-API-Key"
    store: database
```

Insert keys via SQL (no admin API yet):

```sql
INSERT INTO api_keys (client_id, key_hash, scopes, enabled)
VALUES ('my-client', encode(digest('plaintext-secret', 'sha256'), 'hex'),
        'read,write', true);
```

Lookup yields `api_key.client_id` and `api_key.scopes` into the template context.

### Anonymous

Always succeeds; sets subject to `default_subject`. Use for public endpoints (health, login).

```yaml
auth_providers:
  passthrough:
    type: anonymous
    default_subject: "anonymous"
```

## Outbound JWT minting

Used only by `claims_enhancement` hooks with `mint_jwt.enabled: true`. Configuration lives at top-level `jwt:`.

```yaml
jwt:
  signing_algorithm: "HS256"   # HS256|HS384|HS512|RS256|RS384|RS512|ES256|ES384
  signing_key_secret: ""       # HS* — HMAC secret
  signing_key_path:   ""       # RS*/ES* — PEM private key file
  issuer: "https://gate.example.com"
  default_ttl_seconds: 300
```

Env overrides take precedence over YAML and must be used in production for secrets:
- `FLINT_GATE_JWT_SECRET` — HS* HMAC secret
- `FLINT_GATE_JWT_KEY_PATH` — path to PEM for RS*/ES*
- CLI: `--jwt-secret`, `--jwt-key-path`

## JWT signing key rotation

There is no online rotation endpoint; rotation is a config + restart (or hot-reload) operation. The `jwt_signing_keys` table exists in the schema as a placeholder for future online rotation but is not yet used by the signer.

### HS256 rotation (symmetric)

1. Generate a new secret:
   ```bash
   openssl rand -base64 48
   ```
2. Update the source of `FLINT_GATE_JWT_SECRET` (env, secret manager, k8s Secret). Do **not** write it into `config.yaml`.
3. Coordinate with upstream consumers so they accept both the old and new secret during the overlap window, then drop the old one.
4. Apply (rolling restart picks up the new env; hot-reload does not cover env-only changes — a restart is required for env-sourced secrets).
5. Verify a freshly minted token validates against the new secret upstream:
   ```bash
   curl -s -X POST http://localhost:4457/routes/_test_mint \  # if a test route exists
   ```
   Or inspect a real outbound header from a known authenticated request.

### RS256 / ES256 rotation (asymmetric)

1. Generate a new keypair:
   ```bash
   openssl genpkey -algorithm RSA  -out new.pem -pkeyopt rsa_keygen_bits:2048
   # or
   openssl ecparam -name prime256v1 -genkey -noout -out new.pem
   ```
2. Publish the matching public key to the JWKS endpoint your upstreams already use (add the new `kid`, keep the old for the overlap window).
3. Update `FLINT_GATE_JWT_KEY_PATH` (or `jwt.signing_key_path`) to the new PEM.
4. Rolling restart.
5. After upstreams have switched to validate only against the new `kid`, revoke the old key from JWKS.

## Common mistakes

- Setting `issuer`/`audience` on the inbound JWT provider that don't match the token claims — verification fails silently with 401.
- Forgetting that `forward_cookies` only forwards the named `session_cookie`, not all cookies.
- Storing `signing_key_secret` in `config.yaml` checked into git — always use env in production.
- Expecting API keys to work without a row in `api_keys`; the provider returns 401, not 500.
- Hot-reloading after an env change — env overrides are read at process start only. Restart the process.
