# Assessment — agent-identity-and-delegation

_Generated: 2026-07-04 · Phase status: assess_pending · Changes: 0/0 (not yet planned)_

Assessed the flint-gate codebase against the four phase goals. This phase builds
directly on the authz control plane completed last phase, so much of the work is
**extension of existing modules**, not greenfield. Scale: mostly HIGH-feasibility
with two CRITICAL security gates.

## Codebase baseline (what exists to build on)

| Module | Relevant capability | Reuse for this phase |
|--------|--------------------|----------------------|
| `auth/identity.rs` | `Identity { id, traits, schema_id, session_id, aal, extra }` | Principal source; `schema_id`/`extra` are the natural hook for identity-**kind** (G4) |
| `auth/jwt_mint.rs` | `JwtMinter.mint(identity, additional_claims)` — signs JWTs with identity + extra claims | **Direct reuse for RFC 8693** delegated-token issuance (G2): mint a downscoped token carrying an `act` claim |
| `auth/jwt_verify.rs` + `auth/jwks.rs` | JWKS fetch/rotation, issuer/audience/leeway verification | Verify incoming `subject_token`/`actor_token` (G2), introspect JWTs (G3) |
| `auth/mcp.rs` + `mcp_metadata.rs` | RFC 9728 PRM, RFC 8707 audience, `authorization_servers[]` config | AS-discovery + audience binding reusable for token-exchange/introspection (G2/G3) |
| `authz/engine.rs` | `authorize(principal_id, action, resource_id, ctx)`; `make_uid(type_name, id)` **already generic over entity type** | G4 principal-type support is a targeted thread-through, not a rewrite |
| `authz/tool_authz.rs` | per-tool-call gate; principal hardcoded to `User::"<id>"` | G4 must let principal be `Agent`/`Service`, not only `User` |
| `admin/mod.rs` | full admin router (CRUD + analytics + audit + approvals) | G1 adds an authn layer around it |
| `config/types.rs` | `ServerConfig.admin_listen` = `127.0.0.1:4457`; `AuthProviderConfig` enum (Kratos/Jwt/ApiKey/Anonymous/Mcp) | G1 adds `admin_auth`; G2/G3 may add a provider/grant variant |

## Gap analysis (per goal)

### G1 · Admin API authentication — **NOT MET** (CRITICAL, build first)

- **Evidence:** `admin_router()` in `admin/mod.rs` attaches **no** authn middleware
  (`.route(...).fallback(...).with_state(...)` — no `layer` / `from_fn` / auth). The
  only protection is `admin_listen` defaulting to loopback (`config/types.rs:35`,
  documented "unauthenticated and MUST NEVER be internet-exposed").
- **Gap:** the entire control plane — routes/policies/api-keys/signing-keys/approvals
  CRUD **and** the new web UI — has zero request-level authn. Cannot be exposed beyond
  `127.0.0.1` safely; blocks any remote/multi-operator use of the web UI shipped last phase.
- **Shape of fix:** an admin-auth middleware layer + `admin_auth` config (reuse an
  existing `AuthProviderConfig` — JWT or Kratos — or a dedicated admin credential).
  Loopback-dev ergonomics must be preserved (an explicit "no-auth on loopback" opt-in).
- **Feasibility:** HIGH — the auth verification primitives (`jwt_verify`, `kratos`,
  `api_key`) already exist; this wires one of them onto the admin router as a tower layer.
- **Open question:** reuse a configured provider vs a dedicated admin credential/token. → Analyze.

### G2 · OAuth 2.0 Token Exchange (RFC 8693) — **NOT MET** (CRITICAL, strategic core)

- **Evidence:** no token-exchange anywhere. Only a passing comment
  (`auth/mcp.rs:77`, "RFC 8693 / OAuth token responses" re: scope strings). No
  `subject_token` / `actor_token` / `act` / `may_act` / `grant_type` handling.
- **Gap:** an agent cannot exchange a user token for a **downscoped** delegated
  token; it would have to forward the user's raw credential (the confused-deputy
  risk the no-passthrough guard half-addresses).
- **Shape of fix:** a token-exchange endpoint (`grant_type=urn:ietf:params:oauth:
  grant-type:token-exchange`) that verifies `subject_token` (via `jwt_verify`),
  applies scope downscoping, and mints a delegated token with an `act` claim (via
  **`JwtMinter.mint`**). Audience bound per RFC 8707 (reuse existing enforcement).
- **Feasibility:** HIGH-MEDIUM — issuance + verification primitives exist; the new
  surface is the exchange grant, downscoping policy, and `act`/`may_act` claim modeling.

### G3 · Client-credentials + token introspection (RFC 7662) — **NOT MET** (HIGH, S2S identity)

- **Evidence:** no `client_credentials` grant, no `/introspect` endpoint, no
  `grant_type` handling.
- **Gap:** no first-class service-to-service (non-human) identity path, and no way
  for an upstream to introspect an opaque token minted by the gateway.
- **Shape of fix:** a `client_credentials` grant (mint a service token from a
  client_id/secret, reuse `JwtMinter`) + an RFC 7662 `/introspect` endpoint
  (active/scope/aud/exp) backed by `jwt_verify` (and, for opaque tokens, a store).
- **Feasibility:** HIGH — both halves reuse existing mint/verify; mostly new endpoints
  + a client-credential store (extend the api-keys table or a sibling).

### G4 · Workload identity / NHI lifecycle as Cedar principals — **PARTIAL** (HIGH, differentiator)

- **Evidence:** `authz/engine.rs` types **every** principal as `User::"<id>"` via a
  single `PRINCIPAL_TYPE` constant fed to `make_uid`. `tool_authz.rs:13` documents
  `principal → User::"<principal_id>"`. But `make_uid(type_name, id)` is **already
  generic over the entity type**, and `Identity` carries `schema_id`/`extra`.
- **Gap:** an agent/workload identity cannot be a **distinct principal type**
  (`Agent::"…"` / `Service::"…"`) in a Cedar policy — so policies can't say "agents may
  call X but users may not," and there's no issue/rotate/revoke lifecycle for NHI.
- **Shape of fix:** (a) thread a principal **type** (not just id) from `Identity`
  through `authorize`; (b) an identity-kind on `Identity` (user/agent/service);
  (c) a lifecycle store (issue/rotate/revoke) surfaced via Admin API + UI + audit.
- **Feasibility:** MEDIUM — the Cedar plumbing is close (generic `make_uid`); the
  lifecycle store + revocation-takes-effect semantics are the larger part.

## Dependency & build order (recommended)

```
G1 admin authn ─┬─→ (unblocks remote control plane + safe exposure of new web UI)
                │
G2 RFC 8693 ────┼─→ depends on jwt_mint/verify (exist); pairs with no-passthrough guard
                │
G3 client-creds ┤   depends on jwt_mint/verify + a client store; sibling of G2
+ introspection │
                │
G4 NHI/Cedar ───┘   depends on G2/G3 identities existing to name as principals
```

Build **G1 first** (security gate + unblocks the web UI beyond loopback), then G2
(strategic core), G3 (S2S), G4 last (names the G2/G3 identities as Cedar principals).

## Cross-cutting constraints (carry into Spec)

- **Fail-closed everywhere** — the prior phase's recurring defect was auth paths
  degrading to *allow*; every new auth/exchange path needs a `degrades_to_deny`
  test. (Lesson from `agent-authz-control-plane/reflection.md`.)
- **Single-binary** — no new sidecars; stay native-Rust (Cedar, jsonwebtoken@9 pin).
- **Not an IdP** — token-exchange/client-credentials are gateway-scoped, not a
  general authorization server. Federate with Ory/Keycloak/Auth0.
- **Admin authn must not break loopback-dev ergonomics.**

## Open questions for Analyze

1. **Admin authn mechanism:** reuse `JwtAuthConfig`/`KratosAuthConfig` vs a dedicated
   admin credential/token? (affects G1 config shape)
2. **Adopt vs hand-roll RFC 8693/7662:** is there a native-Rust crate that fits the
   jsonwebtoken@9 pin + single-binary constraint, or hand-roll the grants (as MCP RS was)?
3. **Opaque-token store for introspection:** reuse the api-keys table, or a new
   token store? Bearer JWT-only vs opaque support?
4. **Carried from last phase:** Redis-L2 as a hard dep for accurate cross-replica
   budgets — must settle before multi-replica identity scaling.
