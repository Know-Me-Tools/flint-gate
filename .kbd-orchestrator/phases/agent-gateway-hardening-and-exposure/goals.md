# Goals — agent-gateway-hardening-and-exposure

_Seeded from `agent-identity-and-delegation/reflection.md` → "Recommended Next
Phase". The prior phase built the identity/authz **capabilities**; this phase
makes them **safe to expose**, driven directly by that phase's recorded debt._

## Phase Goal

Harden flint-gate's newly-added OAuth / identity surface so it can be safely
exposed beyond a trusted network, and close the identity-classification and
delegation edges left open last phase. Still **authorization-first**; still
outside the LLM-ops bundle; still **federate any JWKS-capable IdM (Ory
Kratos/Hydra reference), never an IdP**.

**Seeded from:** `agent-identity-and-delegation` reflection · **Criteria profile:** effort-impact

## Goals (build order — security-gated, dependency-aware)

1. **Endpoint auth + rate limiting for the OAuth surface** *(prior-phase
   tech-debt #1 — BUILD FIRST; the exposure gate)*. `POST /oauth/token` and
   `POST /oauth/introspect` are currently unauthenticated and unrate-limited on
   the proxy port. Add per-endpoint authentication (client auth per RFC 6749/7662)
   and per-endpoint rate-limiting / failed-attempt backoff **independent of the
   default-off global governor**. Gate the Hydra introspection-delegate behind
   this so it can't be reached unauthenticated. (CRITICAL — nothing else exposes
   safely until this lands.)

2. **Slow-KDF for secrets** — use Argon2/bcrypt for the client-secret hash
   (currently unsalted SHA-256; defensible for a 256-bit CSPRNG token, unsafe for
   anything operator-chosen), and **confirm/enforce the CSPRNG-only insertion
   path** stays the only way to create a client secret. (HIGH — blunts offline
   cracking if the DB leaks.)

3. **Close the identity-classification edges** (prior-phase tech-debt #2/#3/#4):
   - wire API-key identities to `Service` kind (so `Service::` policies apply and
     they're covered by the NHI revocation list);
   - harden the Kratos kind-derivation path (don't derive Agent/Service from
     Kratos `metadata_public`, which some deployments expose to self-service);
   - make NHI lifecycle audit **transactional** (audited-before-effect) rather
     than post-mutation best-effort. (MEDIUM — cross-cutting correctness/compliance.)

4. **RFC 8693 chained delegation + Hydra-delegate exchange** — verify an
   `actor_token` (delegation beyond a single `act`), and wire the
   `delegate_to_hydra` token-exchange seam left defined-but-off last phase.
   (MEDIUM — completes the delegation story; depends on 1–2.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt
  compression, multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- Endpoint-auth mechanism for `/oauth/*`: client-credentials (client auth on the
  token endpoint) vs a dedicated introspection credential (RFC 7662) vs
  network-restriction-only — likely a mix; decide in Assess.
- Redis-L2-as-hard-dep for accurate cross-replica rate limiting on these
  endpoints (carried since two phases ago).

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] `/oauth/token` and `/oauth/introspect` reject unauthenticated/over-rate
      requests; the Hydra-delegate path is unreachable without auth (test-proven).
- [ ] Client secrets are stored under a slow salted KDF; only the CSPRNG path
      can create them.
- [ ] An API-key workload authorizes as `Service::` and is subject to NHI
      revocation; Kratos users can never self-classify as Agent/Service; every
      NHI lifecycle event is audited in the same transaction as its effect.
- [ ] An `actor_token`-bearing exchange is verified (or explicitly rejected); the
      Hydra-delegate token-exchange mode works against a configured Hydra.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new auth path fail-closed (tested).
