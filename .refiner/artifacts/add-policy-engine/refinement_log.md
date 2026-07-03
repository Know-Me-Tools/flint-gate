# Refinement Log — add-policy-engine (QA + security gate)

**Date:** 2026-07-03 · phase agent-authz-control-plane · change 3/8 (strategic core)

## Security review (security-reviewer agent)
Confirmed fail-closed holds end-to-end in-process. Found 1 CRITICAL + 3 HIGH + 1 MEDIUM guardrail. ALL fixed + re-verified independently:
- C1 (CRITICAL): multi-replica hot-reload gap — pg_notify('policies') emitted but listener never reloaded the engine → peers served stale policy. FIXED: authz engine threaded into start_cache_invalidation_listener; Policies=>reload_from_database arm (retain last-good on failure); classify_notification made testable.
- H1 (HIGH): entities_json unvalidated at write → poisoned blob disabled bundle. FIXED: validate_policy now parses entities_json against schema; admin returns 500 stored_but_not_activated when post-write reload fails.
- H2 (HIGH): startup all-or-nothing deny-all blast radius. FIXED: from_records_lenient skips+logs bad rows, builds from survivors; strict from_records retained for write-time validator.
- H3 (HIGH): admin bind default 0.0.0.0 (violates BLOCKING constraint "never expose admin to internet"). FIXED: default now 127.0.0.1:4457 + doc warning. (Full admin authn deferred — noted for reflection.)
- M1 (MEDIUM guardrail): allow-all permit accepted silently. FIXED: policy_warnings + ALLOW_ALL_WARNING non-blocking in 200 response.

## Independent verification (orchestrator, not trusting agent report)
- Verified BEFORE the agent finished that H1/M1 were NOT yet done (validate_policy still returned early on entities; no allow-all helper) — did not prematurely close tasks. Waited; agent completed both.
- Re-ran full suite myself: 232 passed, 0 failed, 3 ignored. Confirmed C1 listener wiring in main.rs (authz built before listener), H1 entities parse in validator, H2 lenient loader, H3 loopback default, M1 detection — all present.
- Confirmed fail-closed intact: lenient loader tests assert degrades_to_deny_not_open; authorize maps every error → Deny.

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Result |
|-----------|--------|
| Admin server not internet-exposed (BLOCKING) | PASS (default now 127.0.0.1:4457) |
| SQL injection (parameterized) | PASS (authz_policies uses $1..$N binds) |
| No unwrap/expect outside tests | PASS (all .expect() are in #[cfg(test)]) |
| No secrets committed | PASS |
| thiserror error types | PASS (AuthzError) |

## Gates
- openspec validate --strict → valid (delta spec: specs/authorization-policy)
- clippy --workspace --all-features -D warnings → clean
- clippy -p flint-gate-core --no-default-features -D warnings → clean
- cargo test --workspace → 232 passed, 0 failed, 3 ignored

## Known follow-up (non-blocking, for reflect)
- Admin endpoints have no authn — network isolation (loopback default) is the only control this change. A dedicated admin auth layer (bearer/mTLS) is a larger scope item for a future change.

## Verdict: PASS — cleared to archive.
