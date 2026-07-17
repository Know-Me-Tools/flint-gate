# Goals — agent-gateway-exposure-operability

_Seeded from `agent-gateway-hardening-and-exposure/reflection.md` →
"Recommended Next Phase". The prior phase made the OAuth/identity surface
**safe to expose**; this phase makes that exposure **operable and observable at
multi-replica scale**, and closes the operability debt that phase documented._

## Phase Goal

Take the now-safe-to-expose OAuth/identity surface and make it **operable at
horizontal scale and observable in production**: give the per-endpoint OAuth
governor and cache/introspection state a real shared cross-replica backend,
add delegate-mode observability, enforce operator guardrails on the exposure
surface, and prove the authenticated exposure paths end-to-end against a real
Ory stack. Still **authorization-first**; still **federate any JWKS-capable IdM
(Ory Kratos/Hydra reference), never an IdP**; LLM-ops bundle stays out of scope.

**Seeded from:** `agent-gateway-hardening-and-exposure` reflection ·
**Criteria profile:** effort-impact

## Goals (build order — dependency-aware)

1. **Shared cross-replica rate-limit + introspection/cache state (Redis-L2 as a
   real backend)** *(carried debt — BUILD FIRST; the horizontal-exposure gate)*.
   The per-endpoint OAuth governor and cache invalidation are currently
   in-process, so multi-replica deployments lack an accurate shared rate-limit /
   introspection-cache store. Wire Redis-L2 as a real backend so rate limits and
   cache state are consistent across replicas. Decide whether Redis-L2 is a hard
   dependency for the exposure posture or a graceful-degrade optional. (CRITICAL
   — nothing exposes *horizontally* accurately until this lands.)

2. **Delegate-mode observability + optional gateway re-stamp** *(prior-phase
   debt #1)*. Delegate-issued tokens currently escape the gateway's
   `flint_kind`/agent-budget classification (Hydra owns RFC 8693). Add metrics
   for delegate success/deny/latency, and decide + implement whether delegated
   tokens should be re-classified / `flint_kind`-re-stamped for agent
   budget/rate-limit parity, or explicitly documented as escaping it. (HIGH —
   closes the observability blind spot on the federation seam.)

3. **Operator guardrails for the exposure surface** *(prior-phase debt #3/#4)*.
   - `https`-only enforcement on `hydra_token_url` / `hydra_admin_url` (reject
     `http://` at startup unless an explicit insecure-dev override is set);
   - a body-size cap on relayed Hydra responses (introspection + token-exchange
     delegate) to bound memory-pressure from a compromised/misbehaving upstream;
   - a startup posture check that **refuses to expose** `/oauth/*` on a
     non-loopback bind without **both** `introspect_auth` and rate-limiting
     configured (fail-safe, mirroring the admin-bind posture). (HIGH — turns the
     documented caveats into enforced invariants.)

4. **End-to-end exposure smoke tests** *(coverage)*. Extend the docker-compose
   smoke stack + Playwright E2E to cover the authenticated `/oauth/token` +
   `/oauth/introspect` + Hydra-delegate paths against a **real Ory stack**
   (Hydra/Kratos), including the fail-closed denials (unauth, over-rate, Hydra
   error/redirect). (MEDIUM — proves the exposure surface behaves under real
   federation, not just unit mocks.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt
  compression, multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- Redis-L2 hard-dependency vs graceful-degrade for the exposure posture: does a
  Redis outage fail the OAuth surface closed (deny) or degrade to in-process
  rate limiting with a logged warning? (Security-vs-availability call — decide in
  Assess.)
- Delegate re-stamp: is re-classifying a Hydra-minted delegated token as an Agent
  identity compatible with "Hydra owns the token", or does it violate the
  federate-first stance? (Decide in Analyze.)
- Metrics backend: Prometheus endpoint on the admin port vs OTLP export — align
  with existing observability wiring.

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] OAuth per-endpoint rate limiting + introspection cache are accurate across
      ≥2 replicas via a shared Redis-L2 backend (test-proven); the Redis-outage
      posture (fail-closed vs degrade) is explicit and tested.
- [ ] Delegate success/deny/latency are observable via metrics; the re-stamp
      decision is implemented or explicitly documented as out-of-scope with a
      recorded rationale.
- [ ] `http://` Hydra URLs are rejected at startup (unless dev-override); relayed
      Hydra response bodies are size-capped; `/oauth/*` on a non-loopback bind
      refuses to start without `introspect_auth` + rate-limiting (all test-proven,
      fail-closed).
- [ ] E2E smoke covers authenticated token/introspect/delegate happy-path + the
      unauth/over-rate/Hydra-error denials against a real Ory stack.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new auth/exposure path fail-closed (tested).
