# Goals — agent-identity-and-delegation

_Seeded from `agent-authz-control-plane/reflection.md` → "Recommended Next Phase"
(deferred workstream D-A05 + prior-phase out-of-scope)._

## Phase Goal

Extend flint-gate's authorization control plane into **non-human-identity (NHI)
and delegation**: let agents act *on behalf of* users and services with scoped,
auditable, revocable identities — building on the MCP resource-server surface
and the Cedar policy engine delivered in the previous phase. Still
**authorization-first**; still deliberately outside the LLM-ops bundle
(semantic caching / multi-LLM routing / multimodal — off-identity).

**Seeded from:** `agent-authz-control-plane` reflection · **Criteria profile:** effort-impact

## Goals (build order — dependency-aware, security-gated)

1. **Admin API authentication** *(pulled forward from prior-phase tech-debt #1 —
   BUILD FIRST)*. The whole control plane (routes/policies/api-keys/approvals +
   analytics + the new web UI) is currently unauthenticated and loopback-only.
   Add authn to the admin router so it can be safely exposed beyond `127.0.0.1`
   and used by remote / multi-operator deployments. Gates everything else here.
   (Prior-phase debt · CRITICAL — unblocks the new web UI beyond loopback)

2. **OAuth 2.0 Token Exchange (RFC 8693)** — `act` / `may_act` delegation so an
   agent acts on-behalf-of a user with a **downscoped** token, never the user's
   raw credential. Extends the confused-deputy prevention already half-built via
   the no-token-passthrough guard. (Deferred D-A05 · CRITICAL — strategic core)

3. **OAuth2 client-credentials + token introspection (RFC 7662)** — service-to-
   service agent identity: mint/verify client-credential tokens and introspect
   opaque tokens for upstreams that need it. (HIGH — S2S identity)

4. **Workload identity / NHI lifecycle** — issue, rotate, and revoke agent
   identities as **first-class principals in Cedar policies** (a policy can name
   an agent identity as principal, not just a user/claim). (HIGH — differentiator)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/failover/LB, prompt
  compression, multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 *authorization server* / IdP (Ory Hydra, Keycloak,
  Auth0 territory — federate with them, don't replace). Token-exchange and
  client-credentials here are gateway-scoped, not a general AS.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- **Redis-L2 as a hard dependency for accurate cross-replica budgets**, vs the
  Postgres windowed `SUM(tokens)` fallback (open from the previous phase; must
  be settled before scaling to multi-replica identity/delegation).
- Admin authn mechanism: reuse an existing auth provider (JWT/Kratos) vs a
  dedicated admin credential — decide in Assess.

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] Admin API rejects unauthenticated requests when authn is enabled;
      loopback-dev mode still ergonomic; web UI works against the authed admin API.
- [ ] An agent exchanges a user token for a downscoped delegated token (RFC 8693
      `act`/`may_act`), and the delegated token authorizes only the reduced scope
      (test-proven); the user's raw credential is never forwarded upstream.
- [ ] Client-credentials tokens mint + verify; opaque-token introspection (RFC
      7662) round-trips for a configured upstream.
- [ ] A Cedar policy names an agent/workload identity as principal and
      allow/deny is enforced per-tool-call; identity revocation takes effect.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`;
      new features ≥80% covered; every authz path fail-closed (tested).
