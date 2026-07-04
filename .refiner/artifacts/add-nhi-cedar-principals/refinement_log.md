# Refinement log — add-nhi-cedar-principals

**Mode:** code-artifact constraint validation
**Date:** 2026-07-04

## Summary
- 6/6 tasks. Distinct Cedar Agent::/Service:: principals (PrincipalKind + authorize_as),
  Identity.kind + claim-derivation, agent_identities lifecycle store (issue/rotate/revoke)
  with fail-closed revocation, Admin API + web UI + audit. Zero new crates.
- 352 core + 5 e2e + others, 0 failed, 6 ignored (DB round-trips). clippy -D warnings clean. SPA typecheck+build ok.
- 10 NHI tests: distinct-type (agent-allowed/user-denied + inverse + service), revocation-denies,
  kind-derivation (act⇒Agent, flint_kind⇒Service, bare-client_id stays User), cedar-type mapping.

## Constraint checks (.kbd-orchestrator/constraints.md)
| Constraint | Severity | Result |
| No secrets committed | BLOCK | PASS |
| Never expose admin (4457) public | BLOCK | PASS (NHI endpoints on protected authed router; startup posture guard) |
| Existing tests not broken | BLOCK | PASS (backward-compat authorize() shim; +10) |
| Parameterized SQL | BLOCK | PASS (agent_identities all bind params) |
| Config priority | BLOCK | PASS (untouched) |
| anyhow/thiserror | WARN | PASS |
| No unwrap/expect outside tests | WARN | PASS |
| Module structure | WARN | PASS |

## Proactive hardening (before review): principal-kind spoofing
Independently identified that the JWT verifier copies ALL non-trait claims (incl. client_id/azp,
which many OIDC user tokens carry) into metadata_public. A bare-client_id⇒Service derivation would
let a human user hit a Service-scoped policy (escalation). FIXED before review: Service/Agent
classification now requires the gateway-stamped, signed `flint_kind` claim (upstream IdPs can't
forge it); minted service tokens set flint_kind=service, delegated tokens flint_kind=agent; bare
client_id no longer implies Service. Tested (bare_client_id_does_not_escalate_to_service).

## Security review (security-reviewer agent) — outcome
Cleared #1 kind-spoofing (metadata_public only claim-verified; flint_kind path sound), #4 Cedar
injection (EntityId is opaque/Infallible, type from 3 constants only), #5 admin endpoints (authed
+ parameterized + kind-validated), #2 revocation defaults, #3 back-compat shim.

**1 HIGH found — FIXED this change:**
- HIGH: the route-level `Authorize` hook (pipeline.rs) still called the User-defaulting authorize()
  shim, coercing Agents/Services to User at the route seam AND skipping revocation. FIXED: route-level
  authorize now threads principal_kind_for(&identity) via authorize_as + a fail-closed NHI revocation
  check (parity with the per-tool gate). Re-tested: 352 core, 0 failed; clippy clean.

Deferred (LOW/INFO, reflection debt): API-key identities not auto-classified Service (fail-closed,
under-scopes — safe); Kratos metadata_public relies on admin-only invariant (safe by default);
NHI audit is post-mutation best-effort not transactional (deny-strengthening, not a hole).

**Verdict: PASS** (HIGH remediated + re-tested; kind-spoofing hardened proactively; core machinery fail-closed).
