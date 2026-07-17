---
type: Reference
id: flint-gate-agent-gateway-hardening-openspec-plan
title: Flint Gate agent gateway hardening OpenSpec plan
tags:
- flint-gate
- agent-gateway
- oauth-hardening
- identity-delegation
- openspec
- rate-limiting
- bcrypt
- hydra
links:
- flint-gate-agent-identity-and-delegation-phase-kickoff
- agent-gateway-hardening-plan-pending-session-ended-without-changes
sources:
- stdin
- manual:flint-gate/agent-gateway-hardening-and-exposure
timestamp: 2026-07-04T19:56:02.780205+00:00
created_at: 2026-07-04T19:56:02.780205+00:00
updated_at: 2026-07-04T19:56:02.780205+00:00
revision: 0
---

## Phase Context

- **Project:** `flint-gate`
- **Phase:** `agent-gateway-hardening-and-exposure`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-gate`
- **Captured:** `2026-07-04T19:48:44Z`
- **Seeded from:** [`agent-identity-and-delegation` reflection](/flint-gate-agent-identity-and-delegation-phase-kickoff.md) recommended next phase
- **Criteria profile:** effort-impact
- **Workflow state:** `plan_pending`
- **Next step:** `/kbd-plan`

## Phase Goal

Harden Flint Gate's newly added OAuth and identity surface so it can be safely exposed beyond a trusted network. The phase closes identity-classification and delegation edges left open by the previous phase.

Constraints and scope:

- Remain **authorization-first**.
- Stay outside the LLM-ops bundle.
- Federate any JWKS-capable IdM, with Ory Kratos/Hydra as the reference.
- Do **not** become an IdP.

## Security-Gated Build Goals

1. **Endpoint auth and rate limiting for OAuth surface**
   - Highest-priority exposure gate.
   - `POST /oauth/token` and `POST /oauth/introspect` are currently unauthenticated and unrate-limited on the proxy port.
   - Add per-endpoint authentication:
     - client auth per RFC 6749 for token endpoint behavior
     - client auth per RFC 7662 for introspection
   - Add per-endpoint rate limiting and failed-attempt backoff independent of the default-off global governor.
   - Gate the Hydra introspection delegate so it is unreachable without authentication.

2. **Slow KDF for secrets**
   - Replace unsalted SHA-256 client-secret hashing with a slow password KDF.
   - Use bcrypt for newly stored client secrets.
   - Preserve compatibility with legacy SHA-256 hashes by format-sniffing at verify time and transparently rehashing after successful legacy verification.
   - Confirm/enforce CSPRNG-only insertion for generated secrets.

3. **Identity classification edges**
   - Classify API-key identities as `Service`.
   - Add Kratos `act` fallback behavior gated off `session_id`.
   - Ensure non-human identity audit behavior is transactional.

4. **Actor token and Hydra delegate behavior**
   - Reject `actor_token` if present instead of silently ignoring it.
   - Wire the Hydra delegate proxy.
   - Fail closed until full actor-token semantics are explicitly supported.

## OpenSpec Changes Written

The backend OpenSpec is complete with 4 ordered changes and 19 tasks. ZeeSpec is inactive.

| Order | Change | Goal | Tasks | Fixed decision |
|---:|---|---|---:|---|
| 1 | `add-oauth-endpoint-hardening` | G1 | 5 | Hard client auth on `/oauth/introspect` per RFC 7662; rate limit `/oauth/token`; apply governor layer to both; Hydra delegate reachable only after auth |
| 2 | `add-bcrypt-secrets` | G2 | 5 | Format-sniff verification: bcrypt for new secrets; legacy SHA-256 verifies and transparently rehashes |
| 3 | `add-identity-classification-edges` | G3 | 5 | API key maps to `Service`; Kratos `act` fallback gated off `session_id`; transactional NHI audit |
| 4 | `add-actor-token-and-hydra-delegate` | G4 | 4 | Reject `actor_token` if present; wire Hydra delegate proxy |

Each change is verified by the `kbd-apply` driver with correct task counts.

## Spec-Level Requirements

- Every change includes an explicit fail-closed `degrades_to_deny` test task.
- OAuth endpoint hardening must re-check authentication at the endpoint seam.
- `/oauth/introspect` authentication is an RFC 7662 **MUST**.
- Existing `actor_token` silent-ignore behavior must become explicit rejection.
- The Hydra introspection delegate must not be reachable without endpoint authentication.

## Dependency Changes

One new crate is planned:

```toml
bcrypt = "0.19"
```

Rationale: deliberate security-justified break from the prior all-reuse approach to replace inadequate secret hashing.

## Open Questions Deferred to Plan/Execute

- **Cross-replica rate limiting:** use `governor` now; Redis-backed distributed windows deferred.
- **Full multi-hop `act` chaining:** deferred until after the fail-closed `actor_token` rejection gate.

## Current State

- `progress.json` lists 4 changes.
- `spec_complete: true`.
- Current stage: `plan_pending`.
- Backend OpenSpec active.
- Driver: `kbd-apply`.
- `spec.handoff.json` written.
- Hook fired.
- Waypoint: `/kbd-plan`.
- Position: `0/4` changes complete.
- Working tree contains the 4 change directories and phase state; not committed.

Related checkpoint: [Agent gateway hardening plan-pending session ended without changes](/agent-gateway-hardening-plan-pending-session-ended-without-changes.md).

# Citations

1. stdin
2. manual:flint-gate/agent-gateway-hardening-and-exposure