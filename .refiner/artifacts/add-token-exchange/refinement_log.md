# Refinement log — add-token-exchange

**Mode:** code-artifact constraint validation
**Date:** 2026-07-04

## Summary
- 6/6 tasks. Gateway-local RFC 8693 token exchange: verify subject_token (any JWKS
  IdM) → downscope → mint delegated token with `act` claim via JwtMinter. Hydra-
  delegate is a defined config seam (off). Zero new crates.
- 17 token_exchange unit/integration tests. Workspace: 319 core + 5 e2e + others, 0 failed.
  clippy --workspace --all-targets -D warnings clean.

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets/keys/creds committed | BLOCK | PASS (config.example uses placeholder secret already flagged "change-me") |
| Never expose admin (4457) to public | BLOCK | PASS (this is a proxy-port endpoint, unrelated) |
| Existing tests not broken | BLOCK | PASS (all prior pass; +17 new) |
| Config priority CLI>env>YAML unchanged | BLOCK | PASS |
| anyhow/thiserror error style | WARN | PASS (typed ExchangeError; anyhow::bail! at wiring) |
| No unwrap/expect outside tests | WARN | PASS |
| Follow existing module structure | WARN | PASS (auth/token_exchange.rs submodule) |

## Fail-closed coverage (degrades_to_deny)
- Scope escalation denied (extra scope, empty-subject-scope-set) — tested.
- Invalid/wrong-grant/malformed subject_token denied before mint — tested.
- Any-JWKS-IdM subject token accepted (vendor-neutral) — tested.
- Minter None → MintFailed (no unsigned token) — covered by exchange() path.

## Security review (security-reviewer agent)
Run separately — see change QA gate outcome appended below.

## Security review (security-reviewer agent) — outcome
Core downscope + minting design confirmed **sound and fail-closed**. Verified safe:
scope-escalation (exact-match subset), alg-confusion + alg:none (jsonwebtoken 9.3.1
family-mismatch backstop + no `none` variant + JWKS refuses `oct` keys), ProviderError
denies (no fail-through to mint), `act` always present, minter-None → MintFailed,
no raw subject_token leak.

**2 HIGH fail-OPEN findings — BOTH FIXED this change:**
- HIGH-1: subject_token_provider accepted non-JWKS providers (anonymous → accepts ANY
  subject_token). FIXED: `validate_subject_provider` startup guard rejects
  anonymous/kratos/api_key; wiring `bail!`s. Tested (incl. the anonymous exploit).
- HIGH-2: plain `jwt` provider without pinned issuer trusts any JWT in its JWKS
  (cross-issuer/audience confused-deputy). FIXED: the guard requires `issuer` pinned on
  a `jwt` subject provider (mcp already fails closed via RFC 8707). Tested.

Deferred (LOW/defense-in-depth, noted for reflection): explicit asymmetric-alg
allowlist on JwtVerifyAuthenticator (proxy-shared; the family-mismatch backstop already
blocks the attack), and optional `allowed_audiences` allowlist for the minted `aud`
(LOW — scopes are still strictly downscoped).

Also fixed a correctness gap the review noted: `scopes_from_identity` now handles both
`scope` (string) and `scp` (string or array) — vendor-neutral across Ory/Auth0/Azure.

**Verdict: PASS** (both HIGHs remediated + re-tested; core design fail-closed).
