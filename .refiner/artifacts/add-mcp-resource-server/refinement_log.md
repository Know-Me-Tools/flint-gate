# Refinement Log — add-mcp-resource-server (QA + security gate)

**Date:** 2026-07-03 · phase agent-authz-control-plane · change 2/8

## Security review (security-reviewer agent)
Found 1 CRITICAL + 2 HIGH + 3 MEDIUM. ALL fixed and re-verified:
- C1 (CRITICAL): audience:None bypass → MCP provider without audience/issuer now fails closed (FailingAuthenticator) in build_authenticators.
- H1 (HIGH): JWKS SSRF → validate_jwks_url (https-only except loopback dev; reject link-local/loopback/private) + dedicated client redirect=none.
- H2 (HIGH): kid downgrade → reject symmetric (oct) JWKs; reject no-kid on multi-key set (asymmetric-only).
- M1: algorithm allowlist (RSA/EC) checked before key resolution.
- M2: JWKS unknown-kid single-flight + MIN_REFRESH_INTERVAL floor (fail-closed).
- M3: issuer required (folded into C1 fail-closed).
Both security-defining requirements confirmed correct: RFC 8707 audience enforcement + no-token-passthrough guard.

## Independent verification (by orchestrator, not trusting agent report)
- Caught 3 failing tests the agent's report missed. Root cause: malformed test-fixture RSA modulus ("Base64 error: Invalid input length: 341"), NOT a production bug — the H2 selector + SSRF validator were correct (proven via probe: validate_jwks_url("https://[fe80::1]") correctly returns ProviderError). Fixed the fixture with a valid 2048-bit modulus.

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Result |
|-----------|--------|
| No secrets/keys committed | PASS (test uses a public RSA modulus) |
| Admin port 4457 not exposed | PASS |
| Existing tests not broken | PASS (191 all-features / 154 no-default) |
| No unwrap/expect outside tests | PASS |
| thiserror error types | PASS (AuthError::InsufficientScope added) |

## Gates
- openspec validate --strict → valid (delta spec: specs/mcp-authorization/spec.md)
- clippy --workspace --all-features -D warnings → clean
- clippy -p flint-gate-core --no-default-features -D warnings → clean
- cargo test --workspace → 191 passed, 0 failed, 3 ignored

## Known follow-up (non-blocking, for reflect)
- OctetKeyPair (OKP/Ed25519) is asymmetric but currently grouped with symmetric-reject. Fails closed (safe), but Ed25519 tokens would be rejected. Note for a future change if Ed25519 AS support is needed.

## Verdict: PASS — cleared to archive.
