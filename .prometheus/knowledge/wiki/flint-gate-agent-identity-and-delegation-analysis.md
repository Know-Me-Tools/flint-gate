---
type: Reference
id: flint-gate-agent-identity-and-delegation-analysis
title: Flint Gate agent identity and delegation analysis
tags:
- agent-identity
- delegation
- oauth-token-exchange
- ory-hydra
- cedar-policy
- admin-authn
- non-human-identity
links:
- agent-authorization-control-plane-executor-session-completion
sources:
- stdin
- manual:flint-gate/agent-identity-and-delegation
- https://datatracker.ietf.org/doc/html/rfc8693
- https://datatracker.ietf.org/doc/html/rfc7662
- https://www.ory.com/docs/hydra/guides/oauth2-token-introspection
- https://github.com/ory/hydra/issues/3723
- https://github.com/ory/hydra/discussions/3359
timestamp: 2026-07-04T15:30:39.473526+00:00
created_at: 2026-07-04T15:30:39.473526+00:00
updated_at: 2026-07-04T15:30:39.473526+00:00
revision: 0
---

## Phase context

- **Project:** `flint-gate`
- **Phase:** `agent-identity-and-delegation`
- **Status:** `spec_pending`
- **Captured:** `2026-07-04T15:17:49Z`
- **Next waypoint:** `/kbd-spec`
- **Seed:** prior `agent-authz-control-plane` reflection, recommended next phase, deferred workstream `D-A05`, and prior-phase out-of-scope work. Related prior execution records include [Agent authorization control plane executor session completion](/agent-authorization-control-plane-executor-session-completion.md).

## Governing identity constraint

flint-gate must **federate any identity management system that can produce a verifiable JWT** using JWKS plus `iss`/`aud` validation. **Ory Kratos/Hydra is the reference standard**, but flint-gate must not become an IdP or OAuth authorization server.

Consequences:

- Authorization remains the product boundary.
- Identity issuance is delegated to external IdM / AS systems.
- Ory compatibility is required as a reference path, not as a lock-in assumption.
- LLM-ops concerns remain out of scope: semantic caching, multi-LLM routing, and multimodal features are off-identity.

## Phase goals

Extend flint-gate's authorization control plane into **non-human identity (NHI) and delegation** so agents can act **on behalf of** users and services with scoped, auditable, revocable identities.

This builds on the MCP resource-server surface and Cedar policy engine delivered in the previous authorization-control-plane phase.

## Build order and gates

1. **Admin API authentication — build first**
   - Pulled forward from prior-phase critical technical debt.
   - Current admin control plane surfaces are unauthenticated and loopback-only:
     - routes
     - policies
     - API keys
     - approvals
     - analytics
     - new web UI
   - Add authentication to the admin router so the control plane can be safely exposed beyond `127.0.0.1` and used in remote or multi-operator deployments.
   - This gates the rest of the phase.

2. **OAuth 2.0 Token Exchange — RFC 8693**
   - Add `act` / `may_act` delegation semantics.
   - Let an agent act on behalf of a user with a **downscoped token**.
   - Never pass through or reuse the user's raw credential.
   - Extends the existing confused-deputy mitigation from the no-token-passthrough guard.

## Analysis outcome

`kbd-analyze` completed for `agent-identity-and-delegation`; state moved to `analysis_complete: true` and `spec_pending`.

Artifacts written in the working tree:

- `analysis.md`
- `library-candidates.json`
- `decision-log.md` with decisions `D-B01` through `D-B04`

No commit was made.

## Build-vs-adopt decisions

Headline decision: **zero new crates; reuse existing primitives**.

| Gap | Verdict | Rationale |
|---|---|---|
| G1 admin-authn | Reuse JWT/Kratos via `from_fn` middleware | Existing verification primitives are already present and tested. Kratos session or Hydra JWT both satisfy the Ory reference path and the general JWT IdM requirement. `axum-login` and `tower-sessions` were rejected because they duplicate identity concerns. |
| G2 RFC 8693 exchange | Federate-first plus gateway-local fallback; hand-roll using existing `JwtMinter` | No suitable Rust server crate was found. Existing ecosystems such as AWS SDK, openai/codex, and warp hand-roll this flow. Local downscoping verifies `subject_token` through JWKS and mints an `act`-claim token, preserving the vendor-neutral guarantee. Hydra delegation remains an optional seam. |
| G3 introspection and client credentials | Hybrid: consume Hydra RFC 7662, hand-roll local introspection for gateway-minted tokens | Hydra natively supports RFC 7662 and client credentials. Duplicating the whole AS feature set would violate the off-identity boundary. Local support is only needed for tokens minted by the gateway. |
| G4 NHI as Cedar principal | Extend the engine to thread principal type and add lifecycle store | Existing `make_uid(type_name, id)` is generic enough for `Agent::` and `Service::` principals. Main implementation work is issue/rotate/revoke lifecycle storage. |

## Ory Hydra findings

Hydra owns RFC 7662 introspection natively and Hydra v2 claims full RFC 8693 support, but real-world delegation issues were found, especially around `aud` handling when exchanging external OIDC tokens.

Relevant Hydra caveats:

- `ory/hydra#3723`: documented `aud` handling problem for external OIDC token exchange.
- `ory/hydra#3359`: related discussion on token exchange behavior.

Design implication: do **not** assume the authorization server always performs RFC 8693 correctly. Use a **federate-first, gateway-local fallback** design:

1. Prefer external IdM / AS federation when available and correct.
2. Verify `subject_token` using JWKS plus issuer/audience checks.
3. Mint gateway-local, downscoped delegation tokens with `act` claims when needed.
4. Keep Hydra-specific delegation as an optional integration seam, not a hard dependency.

This preserves the requirement that any JWT-capable IdM can be used, not only Ory.

## Open questions for specification

- **G2:** Implement Hydra delegation now or gateway-local exchange first?
  - Current lean: gateway-local first.
- **G4:** Model agents as distinct Cedar principal type `Agent::` or as `User` plus `kind`?
  - Current lean: distinct principal type.
- **G3:** Define opaque-token storage behavior for gateway-minted tokens.
- **Admin authn default posture:** allow `none` only on loopback or always require authentication?

# Citations

1. stdin
2. manual:flint-gate/agent-identity-and-delegation
3. https://datatracker.ietf.org/doc/html/rfc8693
4. https://datatracker.ietf.org/doc/html/rfc7662
5. https://www.ory.com/docs/hydra/guides/oauth2-token-introspection
6. https://github.com/ory/hydra/issues/3723
7. https://github.com/ory/hydra/discussions/3359