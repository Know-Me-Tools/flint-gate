# add-identity-classification-edges

## Why
Three identity-classification edges left open last phase (tech-debt #2/#3/#4):
API-key workloads never classify as `Service` (so `Service::` policies don't
apply and they escape the NHI revocation list); the Kratos path can, in a
deployment that exposes `metadata_public` to self-service, let a human
self-classify via an `act` claim; and NHI lifecycle audit is written *after* the
status flip (best-effort, non-transactional). (G3)

## What Changes
Three targeted fixes on existing code (D-C03 — no new deps):
- **API-key → `Service`:** `auth/api_key.rs::build_result` sets
  `Identity.kind = Service` — an API key is a non-human service credential, so it
  authorizes as `Service::"<client_id>"` and is covered by NHI revocation.
- **Kratos self-classification:** `identity.rs::derived_kind` skips the `act`
  fallback when the identity carries a `session_id` (the Kratos marker). The
  gateway-signed `flint_kind` claim (already the trusted signal) is unaffected;
  Kratos `metadata_public` can never promote a kind.
- **Transactional NHI audit:** `revoke`/`issue`/`rotate` write the audit row in
  the **same `sqlx` transaction** as the status change (mirror the `.begin()`
  pattern at `db/mod.rs:834`) — audited-before-effect, not best-effort-after.

## Depends on
- Independent of G1/G2; can land any time after them. Built third.

## Scope
IN: api-key Service classification, Kratos act-fallback gate, transactional NHI
lifecycle audit, tests. OUT: API-key entries in the NHI *lifecycle store* (they
are classified Service for authz but managed via the api-keys admin surface, not
agent-identities); broader Kratos trait modeling.

## Tasks
- [ ] `auth/api_key.rs::build_result`: set `Identity.kind = Service`
- [ ] `identity.rs::derived_kind`: skip the `act` fallback when `session_id` is set (Kratos); `flint_kind` path unchanged
- [ ] Make NHI issue/rotate/revoke + their audit rows write in one DB transaction
- [ ] Tests: api-key identity → Service principal (and subject to revocation); Kratos identity with an `act` trait stays User; audit row committed atomically with a revoke (rollback leaves neither)
- [ ] Docs: note the classification rules; `cargo check/clippy/test --workspace` green
