# Forge Keys — Anon & Service-Role Key Management

Flint Gate is the platform home of the **Forge key-minting utility**:
`scripts/generate-forge-keys.mjs` creates the two long-lived application
credentials every Flint Forge deployment needs, in the Supabase dual-key
model:

| Key | Env var | Safe in clients | JWT `role` | Row-Level Security |
|---|---|---:|---|---|
| Anon key | `FLINT_ANON_KEY` | Yes | `anon` | **Applied** — the key is only as safe as your RLS |
| Service-role key | `FLINT_SERVICE_ROLE_KEY` | **Never** | `service_role` | **Bypassed** (real Postgres `BYPASSRLS`) |

Both are 10-year RS256 JWTs with a `kid` header, `iss`/`aud` =
`flint-forge`, and fixed spec subjects. Forge verifies them directly
(`forge-identity::verify_and_build` → JWKS fetch → signature/issuer/audience
→ the `role` claim becomes the Postgres `SET LOCAL ROLE` for the request).

:::info Why this lives here and not in Forge or an app repo
The utility was originally project-local to one consumer. FFS-001 §11.6
promoted it into flint-gate the moment a second consumer appeared, because
the alternative is copying key-minting code between repositories — and
copied key-minting is how private keys end up in the wrong repo.
:::

## Creating keys

```bash
cd flint-gate
FLINT_PROJECT=acme FLINT_ENV=production node scripts/generate-forge-keys.mjs
```

| Knob (env) | Default | Purpose |
|---|---|---|
| `FLINT_PROJECT` | `flint` | Project slug, baked into the `kid` and `jti` |
| `FLINT_ENV` | `local` | Environment slug — one key set per project × environment |
| `FLINT_KEYS_OUT` | `<repo>/keys` | Output dir for `jwt-private.pem` + `jwks.json` |
| `FLINT_ENV_KEYS_OUT` | `<repo>/.env.keys` | Output path for the token env file |
| `FLINT_JWKS_URL` | `http://127.0.0.1:8787/jwks.json` | The URL written into `.env.keys` for verifiers |

Outputs:

| File | What | Handling |
|---|---|---|
| `keys/jwt-private.pem` | RS256 signing key | `0600`, git-ignored. **Never commit, never serve.** |
| `keys/jwks.json` | Public JWKS (carries the `kid`) | Safe to serve — this is what `FLINT_GATE_JWKS_URL` points at |
| `.env.keys` | Both tokens + matching `FLINT_GATE_*` values | `0600`, git-ignored. Anon key may be published; service key to trusted servers only |

### Why RS256 (and why Gate's runtime HS256 can't do this)

Forge's bearer verification is **JWKS/asymmetric-only**: it requires a `kid`
header and accepts RS256/RS384/RS512/ES256/ES384. A shared HS256 secret has
no public half to publish, so an HS256 token can never pass Forge's
verifier. Gate's own runtime JWT minter (default `HS256`, short TTL) serves
a different job — per-session tokens on auth flows, with identity forwarded
downstream via the trusted `X-Flint-*` headers (see `FLINT-KEYS.md`). The
two long-lived keys documented here bypass that path and are verified from
the token's own `role` claim.

## Serving the JWKS

Verifiers fetch the public half over HTTP. Any static origin works:

```bash
# Local development
npx serve keys                          # → http://127.0.0.1:3000/jwks.json
# or
python3 -m http.server 8787 --directory keys

# Production: any static origin gateways can reach (object storage + CDN is
# fine — it is public material). Keep the URL stable; rotation replaces the
# file's CONTENTS, not its address.
```

Then on every Forge gateway:

```bash
FLINT_GATE_JWKS_URL=https://keys.example.com/jwks.json
FLINT_GATE_ISSUER=flint-forge
FLINT_GATE_AUDIENCE=flint-forge
```

## Using the keys — real-world examples

**Anon key in a browser/mobile client** (publishable — RLS is the boundary):

```ts
// Safe ONLY because every table this key can reach has RLS policies.
const res = await fetch(`${FORGE_URL}/public/articles?status=eq.published`, {
  headers: { Authorization: `Bearer ${FLINT_ANON_KEY}` },
});
```

**Service-role key in a backend job** (bypasses RLS — server-side only):

```bash
# Admin read across all tenants
curl -sS "$FORGE_URL/public/audit_events?order=created_at.desc&limit=100" \
  -H "Authorization: Bearer $FLINT_SERVICE_ROLE_KEY"
```

**Service-role key driving schema provisioning** (Forge's `/schema/v1`
requires exactly this role and refuses everything else with `403`):

```bash
curl -sS -X POST "$FORGE_URL/schema/v1/plan" \
  -H "Authorization: Bearer $FLINT_SERVICE_ROLE_KEY" \
  -H "content-type: application/json" \
  -d @entities.spec.json | tee plan.json | jq -r .ddl   # review, then:

curl -sS -X POST "$FORGE_URL/schema/v1/apply" \
  -H "Authorization: Bearer $FLINT_SERVICE_ROLE_KEY" \
  -H "content-type: application/json" \
  -d "{\"planHash\": \"$(jq -r .planHash plan.json)\"}"
```

See flint-forge's `docs/api/schema-provisioning.md` for the full API.

## Rotation — the revocation path

The keys are 10-year, so **expiry is not a security control; rotation is.**
Re-running the utility regenerates the keypair, both tokens, and a
`jwks.json` with a new `kid`. Once the served JWKS refreshes, old-key
verification fails (unknown `kid`).

:::warning Revocation latency
Forge's JWKS cache is process-global with a `FLINT_GATE_JWKS_TTL_SECS` TTL
(default **600 seconds**) — a rotated-out key keeps verifying on a warm
gateway until the cached set expires. For incident-grade revocation, rotate
**and restart every gateway** (or run with a lower TTL).
:::

Procedure:

```bash
# 1. Keep the old token so you can verify it dies
cp .env.keys .env.keys.pre-rotation

# 2. Rotate
FLINT_PROJECT=acme FLINT_ENV=production node scripts/generate-forge-keys.mjs

# 3. Publish the new keys/jwks.json to the FLINT_GATE_JWKS_URL origin

# 4. Restart every Forge gateway

# 5. Distribute the new .env.keys values to services, then verify:
curl -sS -o /dev/null -w '%{http_code}\n' "$FORGE_URL/schema/v1/status" \
  -H "Authorization: Bearer $OLD_SERVICE_ROLE_KEY"     # expect 401
curl -sS -o /dev/null -w '%{http_code}\n' "$FORGE_URL/schema/v1/status" \
  -H "Authorization: Bearer $NEW_SERVICE_ROLE_KEY"     # expect 200
```

## Best practices

- **One key set per project × environment.** `FLINT_PROJECT`/`FLINT_ENV`
  shape the `kid`/`jti`; a staging leak must never touch production.
- **The service key is a database-superuser-grade secret.** Server-side
  environment/secret manager only. Never a browser, never a log line, never
  a repo. (Forge additionally bounds its provisioning power with a
  per-namespace operator allowlist — defense in depth, not permission to be
  careless.)
- **The anon key is an identity, not a secret** — publish it freely, but
  audit that every table it can reach carries RLS. Forge does not expose
  RLS-less tables through its reflection surfaces at all.
- **Rotate on suspicion, on personnel change, and on cadence** — and always
  pair rotation with gateway restarts (see the latency warning).
- **Serve the JWKS from one stable URL per environment** and treat the
  private PEM's directory as radioactive: `0600`, git-ignored (this repo's
  `.gitignore` covers `keys/` and `.env.keys`), backed up only inside your
  secret manager if at all — regeneration is usually the better recovery.
- **Don't hand-roll variants of this script in app repos.** If you need
  different claims or lifetimes, extend the utility here so every consumer
  inherits the change.

## Relationship to the rest of Gate

- `FLINT-KEYS.md` (repo root of `docs/`) — the key *contract*: accepted key
  shapes, API-key rows, and the trusted `X-Flint-*` header set Gate injects
  after verification.
- [Configuration](configuration.md) — Gate's own `jwt` block
  (`signing_algorithm`, TTLs) for its runtime session minter.
- [Operations](operations.md) — deployment and serving details.
