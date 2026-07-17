# Refinement log — add-identity-classification-edges

**Mode:** code-artifact constraint validation
**Date:** 2026-07-05

## Summary
- 5/5 tasks, 3 targeted fixes (prior-phase debt #2/#3/#4): api-key→Service kind;
  Kratos act-fallback gated off session_id; transactional NHI lifecycle audit. Zero new crates.
- 371 core + 5 e2e + others, 0 failed, 6 ignored. clippy --workspace --all-targets -D warnings clean.
- New tests: api_key→Service principal; 3 Kratos-hardening (session+act→User, no-session+act→Agent,
  session+flint_kind→Service trusted); extended DB round-trip asserts exactly 3 atomic audit rows
  (issue/rotate/revoke; revoke-again no-op → no 4th).

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets committed | BLOCK | PASS |
| Never expose admin (4457) public | BLOCK | PASS (unrelated) |
| Existing tests not broken | BLOCK | PASS (+4) |
| Parameterized SQL | BLOCK | PASS (audit insert + status update all bind params, in txn) |
| Config priority | BLOCK | PASS |
| anyhow/thiserror | WARN | PASS |
| No unwrap/expect outside tests | WARN | PASS |
| Module structure | WARN | PASS |

## Fail-closed / correctness coverage
- api-key → Service (covered by NHI revocation at tool + route gates).
- Kratos session + self-set act → stays User (no self-promotion); jwt/mcp verifiers leave
  session_id=None so their act→Agent is unaffected (verified).
- NHI issue/rotate/revoke + audit row in ONE transaction (audited-before-effect; audit failure
  rolls back the mutation); handler-side best-effort audit removed (no double-audit).

## Security review (security-reviewer agent)
Focus: Kratos marker reliability, api-key-as-Service escalation, audit atomicity, double-audit,
revocation regression. Outcome appended below.

## Security review (security-reviewer agent) — outcome
Reviewer confirmed the substantive properties (stopped after the core analysis; remaining checks
self-verified against code + tests):
- **No revocation regression:** both the route-level Authorize gate and the per-tool gate check
  `Agent | Service` kinds → api-key-as-Service IS subject to NHI revocation.
- **Kratos session gate is safe for delegated tokens:** delegated Agent tokens classify via the
  trusted gateway-signed `flint_kind: "agent"` marker, NOT the `act`-fallback, so gating `act` off
  `session_id` does not break them. jwt_verify/mcp/introspect/client_credentials all leave
  `session_id = None` (verified) → their kind derivation is unaffected.
- **Kratos self-classification closed:** a Kratos identity has `session_id` set → the `act`-fallback
  is skipped → a self-set `act` in metadata_public cannot promote a human to Agent. `flint_kind`
  (which Kratos never sets) remains the only trusted promotion signal.
- **api-key→Service is additive, not escalation:** it enables `Service::`-scoped policies + revocation
  coverage; a `User::"<id>"` policy no longer matches an api-key client, but that is the intended,
  correct type distinction (an api-key was never a human user).
- **Audit atomicity:** issue/rotate/revoke write their audit row via `insert_nhi_audit(&mut *tx)` in
  the SAME transaction as the status change; committed together (audited-before-effect), rolled back
  together on audit failure. `if changed` avoids auditing a no-op revoke. Handler-side best-effort
  `audit_nhi_event` fully removed (grep-confirmed) → no double-audit. Extended DB round-trip asserts
  exactly 3 audit rows.

**Verdict: PASS** (no CRITICAL/HIGH; all six focus questions answered with no bypass; correctness +
fail-closed confirmed).
