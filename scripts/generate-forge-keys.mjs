#!/usr/bin/env node
/**
 * Generate the Flint `anon` and `service_role` keys for a Forge deployment.
 *
 * This is the platform key-minting utility — promoted into flint-gate from
 * the Sansaba workspace per FFS-001 §11.6, so every Forge consumer mints
 * from ONE utility instead of copying key-minting code between repositories
 * (copied key-minting is how private keys end up in the wrong repo).
 *
 * Implements the Supabase dual-key model (`FLINT_ANON_SERVICE_ROLE_KEYS_SPEC.md`
 * §3.1): a publishable `anon` key gated by RLS, and a secret `service_role`
 * key that bypasses it (real Postgres BYPASSRLS on the Forge side).
 *
 * ## Why RS256 rather than flint-gate's HS256 runtime default
 *
 * flint-forge verifies bearers **only asymmetrically**:
 * `forge-identity::verify_and_build` requires a `kid` header, fetches a JWKS,
 * and accepts RS256/RS384/RS512/ES256/ES384 — HS256 is absent. A shared
 * secret has no public half to publish, so an HS256 key can never satisfy
 * Forge's verification path. (Gate's own runtime minter issues short-TTL
 * session tokens and forwards identity via trusted X-Flint-* headers — a
 * different mechanism; see docs/FLINT-KEYS.md.)
 *
 * ## Usage
 *
 *   FLINT_PROJECT=acme FLINT_ENV=production node scripts/generate-forge-keys.mjs
 *
 * Environment knobs (all optional):
 *   FLINT_PROJECT       project slug baked into kid/jti        (default: flint)
 *   FLINT_ENV           environment slug baked into kid/jti    (default: local)
 *   FLINT_KEYS_OUT      directory for pem + jwks               (default: <repo>/keys)
 *   FLINT_ENV_KEYS_OUT  path for the .env.keys file            (default: <repo>/.env.keys)
 *   FLINT_JWKS_URL      value written into .env.keys           (default: http://127.0.0.1:8787/jwks.json)
 *
 * ## Output
 *
 *   <keys>/jwt-private.pem  — signing key. NEVER commit, NEVER serve.
 *   <keys>/jwks.json        — public half; serve this at FLINT_GATE_JWKS_URL.
 *   .env.keys               — the two tokens + matching FLINT_GATE_* values.
 *
 * Re-running regenerates everything, which invalidates previously issued
 * tokens once the served JWKS is refreshed. That IS key rotation and is
 * intentional. NOTE the revocation latency: Forge's JWKS cache TTL
 * (FLINT_GATE_JWKS_TTL_SECS, default 600s) keeps old keys verifying on warm
 * gateways until expiry — rotate AND restart for incident-grade revocation.
 */

import { generateKeyPairSync, createSign } from "node:crypto";
import { mkdirSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const KEY_DIR = process.env["FLINT_KEYS_OUT"] ?? join(ROOT, "keys");
const ENV_PATH = process.env["FLINT_ENV_KEYS_OUT"] ?? join(ROOT, ".env.keys");

const PROJECT = process.env["FLINT_PROJECT"] ?? "flint";
const ENVIRONMENT = process.env["FLINT_ENV"] ?? "local";
const JWKS_URL =
  process.env["FLINT_JWKS_URL"] ?? "http://127.0.0.1:8787/jwks.json";

/** Issuer + audience. Must match FLINT_GATE_ISSUER/AUDIENCE on Forge. */
const ISSUER = "flint-forge";
const AUDIENCE = "flint-forge";
const KID = `${PROJECT}-${ENVIRONMENT}-key`;

/** Ten years (spec §3.1): these carry no user data, so RLS is the boundary, not expiry. */
const TEN_YEARS_SECONDS = 60 * 60 * 24 * 365 * 10;

/** Fixed subjects from the spec. */
const ANON_SUBJECT = "00000000-0000-0000-0000-000000000000";
const SERVICE_SUBJECT = "00000000-0000-0000-0000-000000000001";

const b64url = (input) => Buffer.from(input).toString("base64url");

function sign(payload, privateKey) {
  const header = { alg: "RS256", typ: "JWT", kid: KID };
  const body = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(payload))}`;
  const signer = createSign("RSA-SHA256");
  signer.update(body);
  return `${body}.${signer.sign(privateKey).toString("base64url")}`;
}

function buildJwks(publicKey) {
  const { n, e } = publicKey.export({ format: "jwk" });
  return {
    keys: [{ kty: "RSA", use: "sig", alg: "RS256", kid: KID, n, e }],
  };
}

// ── Generate ────────────────────────────────────────────────────────────────
const { privateKey, publicKey } = generateKeyPairSync("rsa", {
  modulusLength: 2048,
});

const now = Math.floor(Date.now() / 1000);
const exp = now + TEN_YEARS_SECONDS;

/** Publishable. Safe in client code ONLY where RLS policies are in place. */
const anonClaims = {
  sub: ANON_SUBJECT,
  role: "anon",
  principal_type: "User",
  iss: ISSUER,
  aud: AUDIENCE,
  iat: now,
  exp,
  jti: `anon-key-${PROJECT}-${ENVIRONMENT}`,
};

/** Secret. BYPASSES ALL RLS — server-side only, never in a browser. */
const serviceClaims = {
  sub: SERVICE_SUBJECT,
  role: "service_role",
  principal_type: "Service",
  iss: ISSUER,
  aud: AUDIENCE,
  iat: now,
  exp,
  jti: `service-role-key-${PROJECT}-${ENVIRONMENT}`,
};

const anonKey = sign(anonClaims, privateKey);
const serviceRoleKey = sign(serviceClaims, privateKey);

// ── Write ───────────────────────────────────────────────────────────────────
mkdirSync(KEY_DIR, { recursive: true });

writeFileSync(
  join(KEY_DIR, "jwt-private.pem"),
  privateKey.export({ type: "pkcs8", format: "pem" }),
  { mode: 0o600 },
);

writeFileSync(
  join(KEY_DIR, "jwks.json"),
  `${JSON.stringify(buildJwks(publicKey), null, 2)}\n`,
);

writeFileSync(
  ENV_PATH,
  `# Flint anon + service_role keys — GENERATED, DO NOT COMMIT.
#
# Regenerate: node scripts/generate-forge-keys.mjs   (re-running ROTATES)
# Docs: flint-gate Docusaurus → Forge Keys; flint-forge docs/ANON-SERVICE-ROLE-KEYS.md
#
# Project: ${PROJECT} · Env: ${ENVIRONMENT} · Algorithm: RS256 · kid: ${KID}
# Issued:  ${new Date(now * 1000).toISOString()}
# Expires: ${new Date(exp * 1000).toISOString()}
#
# FLINT_ANON_KEY is publishable — safe in client code, but ONLY where RLS
# policies actually constrain it. It is not a secret; it is a scoped identity.
#
# FLINT_SERVICE_ROLE_KEY BYPASSES ALL ROW-LEVEL SECURITY. Server-side only.
# Never ship it to a browser, never log it, never commit it.

FLINT_ANON_KEY="${anonKey}"
FLINT_SERVICE_ROLE_KEY="${serviceRoleKey}"

# Where verifiers fetch the public half (serve ${KEY_DIR}/jwks.json there).
FLINT_GATE_JWKS_URL="${JWKS_URL}"
FLINT_GATE_ISSUER="${ISSUER}"
FLINT_GATE_AUDIENCE="${AUDIENCE}"
`,
  { mode: 0o600 },
);

// ── Report ──────────────────────────────────────────────────────────────────
console.warn(`
Generated Flint keys (RS256, kid=${KID})

  ${join(KEY_DIR, "jwt-private.pem")}   signing key      (0600, git-ignored)
  ${join(KEY_DIR, "jwks.json")}         public JWKS      (safe to serve)
  ${ENV_PATH}                            the two tokens   (0600, git-ignored)

  anon         sub=${ANON_SUBJECT}  role=anon
  service_role sub=${SERVICE_SUBJECT}  role=service_role

WARNING: FLINT_SERVICE_ROLE_KEY bypasses all Row-Level Security. Never expose
it to clients, browsers, or public repositories.

NOTE: rotation takes effect at the served JWKS *plus* the verifier's cache
TTL (FLINT_GATE_JWKS_TTL_SECS, default 600s). Restart gateways for
incident-grade revocation.
`);

if (!existsSync(join(ROOT, ".gitignore"))) {
  console.warn("No .gitignore at the repo root — add one before committing.");
}
