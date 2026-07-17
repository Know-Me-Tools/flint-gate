# Refinement log — add-oauth-endpoint-hardening

**Mode:** code-artifact constraint validation
**Date:** 2026-07-04

## Summary
- 5/5 tasks. RFC 7662 client-auth on /oauth/introspect (Basic + form, verified vs
  oauth_clients store, 401 BEFORE introspect/Hydra-delegate) + per-endpoint governor
  rate-limit on /oauth/*; fail-closed startup guard (introspect_auth+no-db → bail).
  Zero new crates (reuses client store, governor, base64@0.22).
- 361 core + 5 e2e + others, 0 failed. clippy --workspace --all-targets -D warnings clean.
- 9 new oauth tests: 6 credential-extraction (Basic/form/precedence/malformed→None/empty-id),
  3 endpoint-gate (no-creds→401, creds-but-no-store→401, auth-off→introspects).

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets committed | BLOCK | PASS (config.example placeholders) |
| Never expose admin (4457) public | BLOCK | PASS (proxy-port oauth endpoints; Hydra-delegate now gated behind introspect auth) |
| Existing tests not broken | BLOCK | PASS (+9) |
| Config priority CLI>env>YAML | BLOCK | PASS |
| anyhow/thiserror | WARN | PASS (anyhow::bail! startup guard) |
| No unwrap/expect outside tests | WARN | PASS |
| Module structure | WARN | PASS |

## Fail-closed coverage (proactive catch)
- **OAuthConfig Default mismatch caught + fixed:** the derived Default gave introspect_auth=FALSE
  while the serde default is TRUE — a `..Default::default()` construction (tests/config) would have
  silently disabled introspection auth (fail-OPEN). Replaced with an explicit Default (introspect_auth=true)
  so EVERY construction path is fail-closed.
- 401 strictly before introspect/delegate; missing/malformed/empty creds → 401 (tested).
- Startup bail when introspect_auth+no-db (never run an unauthable-but-required endpoint).

## Security review (security-reviewer agent)
Focus: introspection auth-bypass, fail-open default, Basic parsing, rate-limit scope. Outcome appended below.

## Security review (security-reviewer agent) — outcome
**Verdict: PASS (exposure gate met). No CRITICAL/HIGH.** Traced every path:
- **No auth-bypass** at /oauth/introspect — 401 strictly before introspect/delegate; missing/malformed/empty/DB-error creds all → 401; Form-extract failure rejects pre-handler. RFC 7662 §2.1 MUST satisfied.
- **Fail-open default CORRECT** — derived Default removed, explicit Default + serde both yield introspect_auth=true; no construction sets it false implicitly. (My proactive Default-mismatch fix validated.)
- Basic parsing safe (colon-in-password ok, malformed → falls through to form → None); startup guard has no reverse bypass; rate-limit correctly scoped to /oauth/* only; uniform 401 (no client-exists oracle).

Findings + disposition:
- MEDIUM (secret hash is unsalted SHA-256, not bcrypt): this is EXACTLY the next change `add-bcrypt-secrets` (G2) — correctly scoped there. Refiner-log phrasing corrected (SHA-256 today).
- LOW (no test for the fail-closed introspect_auth default): FIXED — added `oauth_introspect_auth_defaults_true_via_struct_default` + `_via_serde_missing_key`.
- LOW (credential-keyed rate-limit doesn't throttle credential-SPRAYING, only reuse): NOTED as reflection debt — add an IP-keyed tier on /oauth/* for enumeration resistance. Inherent to the existing governor.
- LOW positive: delegate_to_hydra now correctly gated behind introspect auth (was the prior-phase M1).

**Verdict: PASS** (exposure gate met; LOW test-gap fixed; MEDIUM is the next change's scope).
