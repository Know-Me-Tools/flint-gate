# Plan — agent-identity-and-delegation

_Generated: 2026-07-04 · Backend: **openspec** · Driver: **kbd-apply** (one task/turn)_
_Changes: 4 · Tasks: 23 · Evolver cycle: no_

## Governing constraint (carried from Analyze)

flint-gate **federates any IdM that produces a verifiable JWT (JWKS + iss/aud);
Ory Kratos/Hydra is the reference standard.** It never becomes an IdP. Every
change reuses existing primitives — **zero new crates** (see
`library-candidates.json`, `new_crates: []`).

## Ordered change list (dependency-aware, security-gated)

| Order | Change | Goal | Verdict (reuse) | Tasks | Depends on |
|-------|--------|------|-----------------|-------|------------|
| 1 | `add-admin-authn` | G1 | reuse JWT/Kratos providers | 5 | — (gates the phase) |
| 2 | `add-token-exchange` | G2 | hand-roll surface + `JwtMinter` | 6 | 1 (config/provider patterns) |
| 3 | `add-client-creds-introspection` | G3 | consume-Hydra + hand-roll local | 6 | 2 (`/oauth/token` + mint patterns) |
| 4 | `add-nhi-cedar-principals` | G4 | extend engine + lifecycle store | 6 | 2, 3 (name the identities they create) |

### Why this order

1. **`add-admin-authn` first — it's the security gate.** The control plane +
   the web UI shipped last phase are currently unauthenticated; nothing else
   should be exposed until this lands. Lowest risk (pure reuse of tested
   verification), highest unblock value.
2. **`add-token-exchange` second — the strategic core.** Establishes the
   `/oauth/token` surface and the delegated-minting pattern the next change
   reuses. Gateway-local-first (operator decision) keeps it vendor-neutral.
3. **`add-client-creds-introspection` third.** Shares the `/oauth/token` surface
   and `JwtMinter` from change 2; adds the S2S grant + RFC 7662 introspection.
4. **`add-nhi-cedar-principals` last.** It names the agent/service identities that
   changes 2–3 create as **distinct Cedar principals** (operator decision), so it
   must follow them.

## Per-change execution notes (for /kbd-apply)

### 1 · add-admin-authn (G1) — `library: reuse auth/jwt_verify.rs + auth/kratos.rs`
- **Recommended agent:** rust-reviewer + security-reviewer (auth-sensitive).
- **Key files:** `config/types.rs` (`AdminAuthConfig`), `admin/mod.rs` (middleware +
  router wiring), server bootstrap (startup posture guard).
- **Decision baked in:** `admin_auth` unset + loopback → allow; unset + non-loopback
  → **refuse start**; set → always enforce. `/health` (+`/ready`) stay open for probes.
- **Gate:** fail-closed test — off-loopback-without-auth must refuse to start.

### 2 · add-token-exchange (G2) — `library: reuse auth/jwt_mint.rs + auth/jwt_verify.rs`
- **Recommended agent:** rust-reviewer + security-reviewer.
- **Key files:** new `auth/token_exchange.rs` (or `oauth/` module), `config/types.rs`
  (`token_exchange` block with `delegate_to_hydra:false` seam), `/oauth/token` route.
- **Decision baked in:** gateway-local only this change; Hydra-delegate is a defined
  config seam, not built. Scope downscoping is subset-check, fail-closed on escalation.
- **Gate:** scope-escalation denied; invalid/expired subject_token denied; any-JWKS
  subject token accepted (vendor-neutral proof).

### 3 · add-client-creds-introspection (G3) — `library: consume Hydra RFC 7662 + reuse JwtMinter`
- **Recommended agent:** rust-reviewer + security-reviewer + database-reviewer (client store).
- **Key files:** client store migration (`oauth_clients` or api-keys extension),
  `/oauth/token` (client_credentials arm), `/oauth/introspect`.
- **Decision baked in:** local introspection for gateway-minted tokens; Hydra-consume
  seam behind config (off). Bearer-JWT-first; opaque store deferred.
- **Gate:** bad client secret denied (constant-time); introspection returns
  `{"active":false}` (never leaks) on malformed/unknown.

### 4 · add-nhi-cedar-principals (G4) — `library: extend authz/engine.rs (cedar-policy@4)`
- **Recommended agent:** rust-reviewer + security-reviewer + typescript-reviewer (UI page).
- **Key files:** `authz/engine.rs` (thread principal type via generic `make_uid`),
  `authz/tool_authz.rs` (call sites), `auth/identity.rs` (`kind`), `agent_identities`
  store + Admin API + web UI + audit.
- **Decision baked in:** distinct Cedar `Agent::`/`Service::` types (not `User`+attr).
- **Gate:** Agent-scoped policy allows Agent but denies User (and vice-versa);
  revoked identity denied on next authorize (fail-closed).

## Cross-cutting (every change)

- **Per-change QA:** artifact-refiner + security-review (all 4 touch authz/identity).
- **Fail-closed everywhere:** each change has an explicit `degrades_to_deny` test task.
- **Workspace-green gate:** `cargo check/clippy -D warnings/test --workspace`; new
  features ≥80% covered.
- **Archive convention:** like last phase, these are code-delta changes with
  `proposal.md`+`tasks.md` (no OpenSpec `specs/` capability delta) → archive via
  `openspec archive --skip-specs --yes`.

## Open questions (non-blocking; resolve in-change)

1. G3 opaque-token store shape (bearer-JWT-first this phase).
2. Carried: Redis-L2 as a hard dep for accurate cross-replica budgets — settle
   before multi-replica identity scaling (not gating changes 1–4).

## First action

`/kbd-apply add-admin-authn` — drive its 5 tasks one per turn.
