# Plan — agent-gateway-hardening-and-exposure

_Generated: 2026-07-04 · Backend: **openspec** · Driver: **kbd-apply** (one task/turn)_
_Changes: 4 · Tasks: 19 · Evolver cycle: no · New crates: 1 (`bcrypt@0.19`)_

## Governing constraint (carried from Analyze)

Harden the OAuth/identity surface for **safe exposure**. Federate any JWKS IdM
(Ory reference); never an IdP; single-binary; jsonwebtoken@9 pin. **Fail-closed +
`degrades_to_deny` on every new/hardened auth path; audit every authorize entry
point** (the two-phase recurring lesson).

## Ordered change list (dependency-aware, security-gated)

| Order | Change | Goal | Verdict | Tasks | Depends on |
|-------|--------|------|---------|-------|------------|
| 1 | `add-oauth-endpoint-hardening` | G1 | reuse client store + governor | 5 | — (exposure gate) |
| 2 | `add-bcrypt-secrets` | G2 | **adopt bcrypt@0.19** | 5 | 1 (shared client-verify path) |
| 3 | `add-identity-classification-edges` | G3 | extend existing | 5 | — (independent; after 1–2) |
| 4 | `add-actor-token-and-hydra-delegate` | G4 | hand-roll reject-first + wire seam | 4 | 1, 2 (OAuth surface hardened first) |

### Why this order

1. **`add-oauth-endpoint-hardening` first — the exposure gate.** `/oauth/*` are
   the only unauthenticated security-sensitive surfaces; nothing else exposes
   safely until introspection auth + rate-limiting land. Pure reuse (client store
   + governor), lowest risk / highest unblock.
2. **`add-bcrypt-secrets` second.** Touches the same client-credentials verify
   path as change 1, so it lands right after — and it's the phase's one new crate,
   isolated to one change.
3. **`add-identity-classification-edges` third.** Three independent small fixes;
   after 1–2 to avoid churn on shared files.
4. **`add-actor-token-and-hydra-delegate` last.** Completes delegation on the
   now-hardened OAuth surface.

## Per-change execution notes (for /kbd-apply)

### 1 · add-oauth-endpoint-hardening (G1) — `library: reuse oauth_clients store + build_governor_layer`
- **Agents:** rust-reviewer + **security-reviewer** (RFC 7662 auth boundary).
- **Files:** `auth/oauth.rs` (introspect client-auth), `main.rs` (governor layer on
  OAuth sub-router), `config/types.rs` (`oauth.rate_limit`, `oauth.introspect_auth`).
- **Decision baked in:** hard client-auth on `/oauth/introspect`; rate-limit on
  `/oauth/token`; Hydra-delegate only reachable authed.
- **Gate:** introspect without/with-bad creds → 401; Hydra-delegate unreachable
  unauthenticated; `degrades_to_deny` on missing/invalid client.

### 2 · add-bcrypt-secrets (G2) — `library: adopt bcrypt@0.19 (NEW crate)`
- **Agents:** rust-reviewer + security-reviewer + database-reviewer (hash/migration).
- **Files:** `Cargo.toml` (bcrypt), `db/mod.rs` (`SecretHash` helper, create/verify
  refactor: fetch-by-id → verify → transparent re-hash of legacy SHA-256).
- **Decision baked in:** format-sniff verify (`$2b$` bcrypt else legacy sha256),
  re-hash legacy on success; CSPRNG-only insertion.
- **Gate:** bcrypt round-trip; wrong secret denied; legacy row verifies + re-hashes;
  `needs_rehash` correct.

### 3 · add-identity-classification-edges (G3) — `library: extend existing`
- **Agents:** rust-reviewer + security-reviewer.
- **Files:** `auth/api_key.rs` (kind=Service), `auth/identity.rs` (Kratos act-gate),
  `db/mod.rs`/`admin/mod.rs` (transactional NHI audit).
- **Decision baked in:** api-key→Service; Kratos `act`-fallback gated off `session_id`;
  NHI issue/rotate/revoke + audit in one transaction.
- **Gate:** api-key→Service principal + revocable; Kratos+`act` stays User; audit
  atomic with revoke (rollback leaves neither).

### 4 · add-actor-token-and-hydra-delegate (G4) — `library: hand-roll + reqwest`
- **Agents:** rust-reviewer + security-reviewer.
- **Files:** `auth/token_exchange.rs` (reject actor_token; Hydra-delegate proxy),
  `main.rs`/config (delegate wiring).
- **Decision baked in:** reject `actor_token` if present (fail-closed, no
  silent-ignore); wire `delegate_to_hydra` proxy, fail closed on Hydra error.
- **Gate:** actor_token present → rejected; absent → normal exchange; delegate
  forwards to Hydra (wiremock) + fails closed on error.

## Cross-cutting (every change)

- **Per-change QA:** artifact-refiner + **security-review** (all 4 touch auth).
- **Fail-closed everywhere:** each change has an explicit `degrades_to_deny` test task.
- **Re-audit every authorize entry point** when an auth path is touched (last phase's
  route-level-shim miss).
- **Workspace-green gate:** `cargo check/clippy -D warnings/test --workspace`; ≥80% new-code coverage.
- **Archive convention:** code-delta changes with `proposal.md`+`tasks.md` (no OpenSpec
  `specs/` delta) → `openspec archive --skip-specs --yes`.

## Open questions (non-blocking; resolve in-change)

1. Cross-replica rate limiting — governor (per-replica) this phase; Redis window
   counters deferred (carried Redis-L2 question).
2. Full multi-hop `act` chaining — deferred after the reject-if-present gate (G4).

## First action

`/kbd-apply add-oauth-endpoint-hardening` — drive its 5 tasks one per turn.
