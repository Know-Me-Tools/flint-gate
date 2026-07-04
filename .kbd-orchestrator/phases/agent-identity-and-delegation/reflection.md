# Reflection — agent-identity-and-delegation

_Generated: 2026-07-04 · Phase status: **COMPLETE** (4/4 changes archived)_

## Phase Goal

Extend flint-gate's authorization control plane into **non-human-identity (NHI)
and delegation** — letting agents act *on behalf of* users and services with
scoped, auditable, revocable identities — building on the MCP resource-server
surface and Cedar policy engine from the prior phase. Governed by the operator
directive: **federate any IdM that produces a verifiable JWT; Ory Kratos/Hydra is
the standard; flint-gate never becomes an IdP.**

## Goal Achievement

| # | Goal | Status | Evidence |
|---|------|--------|----------|
| 1 | Admin API authentication | ✅ **MET** | `add-admin-authn`: reuse JWT/Kratos via tower middleware; fail-closed startup posture (loopback-dev allowed, non-loopback-without-auth refuses to start); `/health`+`/ready` stay open. Integration test proves the SPA fallback is authed. |
| 2 | RFC 8693 token-exchange (delegation) | ✅ **MET** | `add-token-exchange`: gateway-local downscoping exchange (any-JWKS subject → `act`-claim delegated token via JwtMinter); scope escalation denied; raw token never forwarded. Hydra-delegate seam defined (off). |
| 3 | Client-credentials + RFC 7662 introspection | ✅ **MET** | `add-client-creds-introspection`: `client_credentials` grant (hashed-secret store) + local RFC 7662 introspection for gateway-minted tokens + Hydra-consume seam. |
| 4 | NHI as Cedar principals + lifecycle | ✅ **MET** | `add-nhi-cedar-principals`: distinct `Agent::`/`Service::` Cedar types (Agent-scoped policy allows Agent, denies User — and inverse); `agent_identities` issue/rotate/revoke store; fail-closed revocation on next authorize; Admin API + web UI + audit. |

**Goal completion: 4/4 MET (100%).**

### Success-criteria checklist (from goals.md)

- [x] Admin API rejects unauthenticated requests when authn enabled; loopback-dev ergonomic; web UI works authed.
- [x] Agent exchanges a user token for a **downscoped** delegated token (RFC 8693 `act`); reduced scope test-proven; raw credential never forwarded.
- [x] Client-credentials tokens mint + verify; RFC 7662 introspection round-trips (local + Hydra-delegate seam).
- [x] A Cedar policy names an agent/workload identity as principal, allow/deny enforced; **revocation takes effect on next authorize** (tested).
- [x] Workspace green — `cargo check/clippy -D warnings/test --workspace`: **352 core unit + 5 MCP-e2e + 16 + 11 + 4 + 1 doc, 0 failed**, 6 ignored (DB round-trips gated on `DATABASE_URL`). Every authz path fail-closed (tested).

## Delivered Changes (dependency order)

| Order | Change | Goal | Tasks | Archived |
|-------|--------|------|-------|----------|
| 1 | `add-admin-authn` | G1 | 5/5 | 2026-07-04 |
| 2 | `add-token-exchange` | G2 | 6/6 | 2026-07-04 |
| 3 | `add-client-creds-introspection` | G3 | 6/6 | 2026-07-04 |
| 4 | `add-nhi-cedar-principals` | G4 | 6/6 | 2026-07-04 |

**Zero new crates** — every change reused existing primitives (`JwtMinter`,
`jwt_verify`, `cedar-policy@4`, the api-keys store pattern), honoring the
Analyze-stage all-reuse plan.

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA (artifact-refiner + security-review) | 4/4 (100%) |
| First-pass clean (no HIGH/CRITICAL) | 1/4 (`add-admin-authn`) |
| Changes with HIGH findings fixed mid-QA | 2/4 |
| **CRITICAL findings** | **0** |
| **HIGH fail-open/escalation defects caught + fixed pre-archive** | **5** |
| Post-fix test suite | 352 core + 5 e2e + others, 0 failed |

### The security-review catch record (all fixed before archive)

- **`add-token-exchange` (2 HIGH):** `subject_token_provider` accepted a
  fail-open provider (an `anonymous` provider would accept *any* subject_token);
  a `jwt` provider without a pinned issuer trusted any JWT in its JWKS. Fixed
  with a `validate_subject_provider` startup guard (jwt/mcp + issuer required).
- **`add-client-creds-introspection` (M1 mitigated + M3/L1 fixed):** the
  unauthenticated `/oauth/introspect` + Hydra delegate could proxy to Hydra's
  admin API — mitigated with a config SECURITY warning; scope dedup + generic
  5xx (no DB-error leak) fixed.
- **`add-nhi-cedar-principals` (1 HIGH + proactive hardening):** I proactively
  closed a **principal-kind spoofing** vector before review (bare `client_id`
  claim — common in OIDC user tokens — would have classified a human as a
  Service; now requires the gateway's signed `flint_kind` marker). The review
  then found a HIGH I'd missed: the **route-level `Authorize` hook** still
  coerced NHIs to `User` and skipped revocation — fixed to thread the real kind
  via `authorize_as` + a fail-closed revocation check, at parity with the
  per-tool gate.

### Recurring constraint pattern

**No recurring *constraint* violations** — all four changes PASSED the blocking
constraints (no secrets, admin not exposed, parameterized SQL, tests intact).
The recurring *theme* was **auth degrading to fail-OPEN through a
configuration/seam gap**, not the core logic — exactly the prior phase's lesson,
and every instance was caught and closed.

## Technical Debt Introduced

1. **`/oauth/token` + `/oauth/introspect` are unauthenticated on the proxy port,
   with no per-endpoint rate limiting** (default governor is off). The
   client-credentials guessing surface and the introspection oracle both want
   endpoint auth + rate-limiting. **Top follow-up** (from `add-client-creds`
   H1/M1). `create_oauth_client` always uses a 256-bit CSPRNG secret, so the
   practical risk today is low.
2. **API-key identities are never classified as `Service`** — they authorize as
   `User::"<client_id>"` and escape the NHI revocation list. Fail-closed
   (under-scopes), but a correctness gap for operators writing `Service::`
   policies for API-key clients.
3. **Kratos-path kind derivation** relies on Kratos `metadata_public` being
   admin-API-writable only. Safe by default; a deployment that exposes
   `metadata_public` to self-service could let a human self-classify. A
   defensive early-return would harden it.
4. **NHI lifecycle audit is post-mutation + best-effort** (not transactional).
   Deny-strengthening, so not a security hole, but "audited-before-effect"
   compliance would need same-transaction writes.
5. **`add-web-config-ui` archive convention** carried over: all changes use
   `proposal.md`+`tasks.md` (no OpenSpec `specs/` delta) → archived
   `--skip-specs`. A backend *convention* choice, not a defect.

## Lessons Captured (for the knowledge base)

- **Separated security-review remains the single highest-value gate on authz
  work.** Two phases running, the pattern holds: the agent that writes the code
  never sees its own fail-open seams. This phase, review caught 5 HIGH
  escalation/fail-open defects the implementer's tests passed — and in the NHI
  change it caught a route-level gap *after* I'd already proactively hardened a
  different spoofing vector. Neither of us alone would have shipped it safe.
- **"Fail-open through configuration" is the dominant authz failure mode.** The
  core logic was fail-closed every time; the holes were in the *seams* —
  provider selection, an unpinned verifier, a User-defaulting back-compat shim
  at a second authorize site. **Lesson: every authorize *entry point* must be
  audited, not just the engine** — a shim that safely defaults is a landmine at
  a call site that should pass a real value.
- **Kind/type from claims must be forgery-resistant.** Deriving a security-relevant
  *type* from a raw token claim (`client_id`) that untrusted issuers also emit is
  an escalation vector. The fix — a **gateway-minted, signed marker** (`flint_kind`)
  — is the reusable pattern: derive privilege from what *you* signed, not what the
  token merely contains.
- **Federate-first with a local fallback delivered the vendor-neutral promise.**
  Every grant/introspection has a "prefer the configured AS (Hydra)" seam plus a
  gateway-local path, so "any IdM with a verifiable JWT" is real, not Ory-only.
- **Backward-compatible engine extension worked.** Adding `authorize_as(kind, …)`
  with `authorize(…)` as a `User` shim let the whole codebase keep compiling while
  the new principal types landed — but see the shim-landmine lesson above.

## Recommended Next Phase

**`agent-gateway-hardening-and-exposure`** — turn the now-complete identity/authz
control plane into something safely *exposable*, driven by this phase's debt:

1. **Endpoint auth + rate limiting for the OAuth surface** (tech-debt #1) —
   authenticate `/oauth/token` and `/oauth/introspect`, add per-endpoint
   rate-limiting/backoff independent of the default-off global governor. This is
   the gate for exposing the OAuth endpoints beyond trusted networks. *Do first.*
2. **Argon2/bcrypt for operator-settable secrets** + confirm the CSPRNG-only
   client-secret path stays the only insertion route.
3. **Close the identity-classification edges** — API-key→`Service` wiring
   (tech-debt #2), the Kratos self-service hardening (#3), and transactional NHI
   audit (#4).
4. **RFC 8693 chained delegation** (`actor_token` verification beyond a single
   `act`) + Hydra-delegate token-exchange mode (the seam left off this phase).

Stay authorization-first; still avoid the LLM-ops bundle. This phase built the
*capabilities*; the next makes them *safe to expose*.
