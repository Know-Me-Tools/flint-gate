# Refinement log — add-bcrypt-secrets

**Mode:** code-artifact constraint validation
**Date:** 2026-07-05

## Summary
- 5/5 tasks. Adopt bcrypt (the phase's one new crate) for oauth client secrets;
  format-sniff verify + transparent re-hash of legacy SHA-256 rows. Verify refactored
  from hash-equality lookup to fetch-by-client_id + KDF verify (bcrypt's per-hash salt
  precludes a WHERE secret_hash lookup).
- 366 core + 5 e2e + others, 0 failed, 6 ignored (DB round-trips incl. the new legacy-migration case).
  clippy --workspace --all-targets -D warnings clean.
- New tests: 3 SecretHash pure (bcrypt round-trip + salt-uniqueness, legacy verify + needs_rehash,
  never-panics-on-garbage) + extended DB round-trip (fresh=bcrypt, legacy verifies + upgrades to bcrypt).

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets committed | BLOCK | PASS |
| Never expose admin (4457) public | BLOCK | PASS (unrelated) |
| Existing tests not broken | BLOCK | PASS (+6) |
| Parameterized SQL | BLOCK | PASS (all bind params; active=true filter preserved) |
| Config priority | BLOCK | PASS |
| anyhow/thiserror | WARN | PASS (bcrypt errors via anyhow::Context) |
| No unwrap/expect outside tests | WARN | PASS (bcrypt::verify unwrap_or(false) — fail-closed) |
| Module structure | WARN | PASS |
| Argon2/bcrypt for secrets (rust security rule) | WARN | ADDRESSED — this change moves off sha256 to bcrypt |

## Fail-closed coverage
- Wrong secret / unknown client → Ok(None) → deny (tested).
- bcrypt::verify errors → unwrap_or(false) → deny.
- Legacy sha256 rows verify + upgrade to bcrypt; re-hash is best-effort (auth succeeds regardless).
- `active = true` filter preserved (revoked client denied).

## Security review (security-reviewer agent)
Focus: timing enumeration, bcrypt 72-byte truncation, re-hash race, active-filter. Outcome appended below.

## Security review (security-reviewer agent) — outcome
**Verdict: PASS. No CRITICAL/HIGH.** All fail-closed invariants confirmed vs bcrypt 0.19.2 source:
wrong-secret/unknown/inactive → deny; no format-sniff bypass (empty/malformed hash → deny);
bcrypt::verify Err → unwrap_or(false); `active=true` filter preserved; no TOCTOU corruption on
concurrent re-hash (last-writer-wins, both valid). Null-byte safe (bcrypt 0.19 hashes as data).

Findings + disposition:
- MEDIUM (bcrypt silent 72-byte truncation on unbounded verify input — latent, not exploitable today
  since all real secrets are 64B): FIXED — SecretHash::hash now rejects >72B (bail), test added. Makes
  the invariant enforced rather than incidental.
- LOW (stale doc comment claiming SHA-256-hash-lookup): FIXED — corrected to fetch-by-id + KDF-verify.
- MEDIUM (best-effort re-hash can silently strand a row on SHA-256 if UPDATE persistently fails): NOTED
  as reflection debt — add a re-hash-failure metric / periodic "clients still on legacy hash" audit query.
  Monitoring gap, not an auth flaw.
- LOW (client_id enumeration by verify timing): ACCEPTED — client_id is a non-secret OAuth identifier;
  optional dummy-hash mitigation noted.
- LOW (legacy SHA-256 == non-constant-time): ACCEPTED — compares digests not secrets; self-eliminating.

**Verdict: PASS** (2 MEDIUM/LOW fixed + re-tested; remaining items = monitoring/defense-in-depth reflection debt).
