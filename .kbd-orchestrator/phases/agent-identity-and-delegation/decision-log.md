# Decision Log — agent-identity-and-delegation

### 2026-07-04 — Analyze: build-vs-adopt calls (stack-specified; governed by "federate any JWT-capable IdM, Ory is standard")

- **D-B01 · Admin authn → REUSE existing auth providers** (JWT/Kratos via `from_fn` middleware). [analyze · 2026-07-04]
  TL;DR: admin router has zero authn; wire a tower layer that verifies against a configured `AuthProviderConfig`.
  Why: verification primitives exist + tested; no new crate/identity model; single-binary preserved; Kratos/Hydra-JWT both satisfy "Ory is standard" and "any IdM with a JWT."
  Alternatives: axum-login/tower-sessions (rejected — duplicate identity + session store), bespoke admin password (rejected — off-standard). Provenance: research + user directive.
- **D-B02 · RFC 8693 token-exchange → FEDERATE-FIRST + gateway-local fallback** (hand-roll surface, reuse `JwtMinter`). [analyze · 2026-07-04]
  TL;DR: no Rust server crate exists (AWS SDK / openai-codex / warp all hand-roll); verify `subject_token` via JWKS → mint downscoped delegated token with `act` claim; proxy to Hydra where configured.
  Why: gateway-local downscoping is the vendor-neutral guarantee for "any IdM with a JWT"; delegate-mode uses Hydra's native 8693 (with known external-`aud` caveats, ory/hydra#3723). Mirrors last phase's D-A03 hand-roll pattern.
  Alternatives: oauth2/openidconnect (rejected — client-only), full AS (rejected — off-identity). Provenance: research (Tier 1 gh + Tier 4 Ory docs/issues).
- **D-B03 · Client-credentials + RFC 7662 introspection → HYBRID** (consume Hydra + hand-roll local for gateway-minted tokens). [analyze · 2026-07-04]
  TL;DR: Hydra owns both natively; flint-gate adds only local introspection for its own minted tokens + a federation seam preferring Hydra.
  Why: duplicating Hydra wholesale is off-identity. Provenance: research (Ory introspection docs).
- **D-B04 · NHI as Cedar principal → EXTEND engine (thread principal type) + lifecycle store.** [analyze · 2026-07-04]
  TL;DR: `make_uid(type_name,id)` already generic → add `Agent::`/`Service::` principals; add identity-kind on `Identity`; add issue/rotate/revoke store surfaced via Admin API + UI + audit.
  Why: Cedar plumbing close; real work is lifecycle store + revocation semantics; no new engine.
  Open: distinct Cedar `Agent::` type vs `User`+`kind` attribute (lean: distinct type). Provenance: codebase assessment.
