# Refinement log — add-client-creds-introspection

**Mode:** code-artifact constraint validation
**Date:** 2026-07-04

## Summary
- 6/6 tasks. OAuth2 client_credentials grant + RFC 7662 introspection (gateway-minted
  tokens) + Hydra-delegate seam; unified `/oauth/token` grant dispatcher + `/oauth/introspect`.
  `oauth_clients` store (SHA-256 hashed secret). Zero new crates.
- 340 core + 5 e2e + others, 0 failed, 5 ignored (DB round-trips gated on DATABASE_URL).
  clippy --workspace --all-targets -D warnings clean.
- New tests: 7 client_credentials, 6 introspect (incl. wrong-secret/wrong-issuer inactive,
  Hydra delegate + fail-closed), 1 sha256_hex, 1 ignored DB round-trip (create/verify-good/
  bad-secret/unknown-client).

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets/keys committed | BLOCK | PASS (config.example placeholders; raw client secret returned once, only hash stored) |
| Never expose admin (4457) to public | BLOCK | PASS (these are proxy-port endpoints) |
| Existing tests not broken | BLOCK | PASS (+14 new) |
| Parameterized SQL (no injection) | BLOCK | PASS (all sqlx bind params; client_id/secret bound) |
| Config priority CLI>env>YAML | BLOCK | PASS |
| anyhow/thiserror error style | WARN | PASS (typed ClientCredentialsError; anyhow at wiring) |
| No unwrap/expect outside tests | WARN | PASS |
| Follow module structure | WARN | PASS (auth/client_credentials.rs, introspect.rs, oauth.rs) |

## Fail-closed coverage
- client_credentials: wrong secret / unknown client → InvalidClient (no token); scope ⊆ grant.
- introspection: garbage / wrong-secret / wrong-issuer → {"active":false} (leaks nothing).
- Hydra delegate: transport error / non-200 → {"active":false}.
- grant dispatch: disabled grant → unsupported_grant_type (cannot execute).

## Known limitation (documented)
- `/oauth/introspect` is unauthenticated. RFC 7662 §2.1 recommends auth on the endpoint.
  Mitigated: it only introspects gateway-minted tokens locally and returns active:false for
  anything else, so it is not a general token oracle. Noted for reflection (add endpoint
  auth / rate-limit if introspection is exposed beyond trusted callers).

## Security review (security-reviewer agent)
Run separately — outcome appended below.

## Security review (security-reviewer agent) — outcome
**No CRITICAL.** All fail-closed invariants verified (unknown/wrong-secret→invalid_client,
scope⊆grant, unverifiable/wrong-issuer/wrong-sig→active:false, only gateway-minted active,
service-token sub=client_id). All 6 focus questions cleared: alg-confusion blocked (alg pinned
to configured), no fail-open on Hydra outage, disabled grant cannot execute, empty secret can't
match, SQL fully parameterized.

Findings + disposition:
- H1 (client-secret KDF + unauth/unrate-limited /oauth/token brute-force surface): SHA-256 of a
  256-bit CSPRNG secret is safe against guessing; the residual risk is an operator seeding a weak
  secret + no endpoint rate-limit. NOTED as reflection debt (endpoint auth + per-endpoint rate
  limit + argon2/bcrypt for any operator-chosen secret). create_oauth_client is the only insertion
  path and always uses CSPRNG.
- M1 (unauth /oauth/introspect oracle; delegate → Hydra admin proxy): MITIGATED — added a SECURITY
  warning on introspection_delegate config + README note; full endpoint auth roadmapped (reflection
  debt). The local oracle only reflects tokens the caller already holds.
- M2 (validate_aud=false on introspect): CONFIRMED CORRECT (introspection = liveness, not authz).
- M3 (scope dedup): FIXED + tested.
- L1 (MintFailed leaks DB error): FIXED — 5xx now returns generic "internal server error", detail
  logged server-side.
- L2/L3 (Hydra URL safe, SQL parameterized): CONFIRMED clean, no change.

**Verdict: PASS** (no CRITICAL; M3+L1 fixed + re-tested; M1 mitigated w/ warning; H1/M1-auth
recorded as reflection debt — the endpoint-auth model is a deliberate cross-change decision).
