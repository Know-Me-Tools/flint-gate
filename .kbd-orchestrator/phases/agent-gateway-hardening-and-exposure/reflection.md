# Reflection — agent-gateway-hardening-and-exposure

_Phase closed: 2026-07-06 · Backend: openspec · Driver: kbd-apply_
_Seeded from: `agent-identity-and-delegation/reflection.md`_

## Phase Goal (restated)

Make the identity/authz control plane built last phase **safe to expose** beyond
a trusted network: authenticate + rate-limit the OAuth surface, slow-KDF the
client secrets, close the identity-classification edges, and finish the
delegation story (`actor_token` + Hydra-delegate). Still authorization-first;
still **federate any JWKS-capable IdM (Ory Kratos/Hydra reference), never an
IdP**; LLM-ops bundle stays out of scope.

## Goal Achievement

| Goal | Status | Evidence |
| --- | --- | --- |
| **G1 — OAuth endpoint auth + rate limiting** | ✅ MET | `add-oauth-endpoint-hardening`: `introspect_auth` (RFC 7662 §2.1) gates `/oauth/introspect` on client-credentials (Basic + form, Basic precedence); per-endpoint `oauth.rate_limit` governor independent of the default-off global one; Hydra introspection-delegate now unreachable without auth (401 before delegate). Startup guard bails when `introspect_auth` + no DB. |
| **G2 — Slow-KDF for secrets** | ✅ MET | `add-bcrypt-secrets`: `SecretHash` = bcrypt with a 72-byte truncation guard + format-sniff verify of legacy SHA-256 + `needs_rehash` transparent re-hash on next successful auth; CSPRNG-only insertion path preserved (`active=true` filter intact, all bind params). |
| **G3 — Identity-classification edges** | ✅ MET | `add-identity-classification-edges`: API-key identities → `IdentityKind::Service` (Service policies + NHI revocation coverage); Kratos kind-derivation no longer promotes to Agent/Service off self-service `metadata_public` (gated on `session_id.is_none()` + signed `flint_kind`); NHI lifecycle audit made transactional (audited-in-tx via `insert_nhi_audit`). |
| **G4 — actor_token + Hydra-delegate** | ✅ MET | `add-actor-token-and-hydra-delegate`: a present `actor_token` fails closed (`400 invalid_request`) *before* both the delegate branch and local mint; `delegate_to_hydra` proxies the RFC 8693 exchange to a configured Hydra endpoint, fail-closed on transport/non-2xx/bad-JSON, with a no-redirect delegate client (SSRF/token-exfil guard). |

**4/4 goals MET (100%).** Every draft success-criterion checkbox in `goals.md`
is satisfied and test-proven.

## Delivered Changes

| # | Change | Goal | Tasks | Status |
| --- | --- | --- | --- | --- |
| 1 | `add-oauth-endpoint-hardening` | G1 | 5/5 | archived (`2026-07-05`) |
| 2 | `add-bcrypt-secrets` | G2 | 5/5 | archived (`2026-07-05`) |
| 3 | `add-identity-classification-edges` | G3 | 5/5 | archived (`2026-07-06`) |
| 4 | `add-actor-token-and-hydra-delegate` | G4 | 4/4 | archived (`2026-07-06`) |

Build order followed the security-gated plan exactly: G1 (the exposure gate)
first, then G2, G3, G4 (which depended on 1–2). Verification gate met per change:
`cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test
--workspace` green (378 core tests at phase end, +24 net new across the phase).

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA | 4/4 (100%) |
| First-pass pass rate | 4/4 (100%) — all PASS, none marked BLOCKED |
| Changes requiring refinement iteration | 0 (fixes applied inline, pre-archive) |
| Blocking-constraint violations | 0 across all 4 changes |
| Security-review HIGH/CRITICAL surviving to archive | 0 |

### Blocking constraints — all PASS every change

`no-secrets`, `admin-4457-not-public`, `no-broken-tests`, `config-priority
CLI>env>YAML`, and (where SQL touched) `parameterized-SQL` all passed as BLOCK-
level in all four refinement logs. No recurring violation pattern — nothing
failed even once.

### Security findings caught & remediated *before* archive

The separated security-review discipline (author never grades its own fail-open
seams) paid out again:

- **G1** — `OAuthConfig` derived `Default` gave `introspect_auth = false` while
  the serde default is `true` — a fail-**open** mismatch. Caught proactively,
  fixed with an explicit `Default` impl (fail-closed).
- **G2** — bcrypt 72-byte silent-truncation risk → explicit length guard; stale
  "unsalted SHA-256" doc corrected.
- **G3** — principal-kind spoofing via a bare `client_id` → hardened to require a
  gateway-signed `flint_kind`; Kratos self-service promotion path closed.
- **G4** — **HIGH: delegate client followed HTTP redirects**, so a
  compromised/tricked Hydra 3xx could exfiltrate the raw `subject_token` to an
  attacker host. Fixed with a dedicated `redirect(Policy::none())` client (mirror
  of the existing JWKS-client guard); a 3xx now surfaces as non-2xx → deny.
  Covered by `delegate_fails_closed_on_hydra_redirect`. Two LOWs (delegate drops
  gateway downscope/`flint_kind`; unbounded Hydra body relay) accepted as
  documented by-design federation semantics.

## Technical Debt Introduced

1. **Delegate mode is un-downscoped from the gateway's view** — in
   `delegate_to_hydra`, the gateway's local downscope + `flint_kind=agent`/`act`
   stamping are skipped (Hydra owns RFC 8693). Correct by design, but delegated
   tokens escape the gateway's Agent classification, so downstream agent
   budget/rate limits keyed on `flint_kind` won't apply to them. Documented;
   revisit if delegate-mode tokens must carry gateway agent-budget semantics.
2. **ory/hydra#3723 external-`aud` quirk** — Hydra's audience handling when
   exchanging *external* tokens is a documented operator-config caveat, not code
   we control. Flagged in `config.example.yaml` + `config/types.rs`.
3. **Redis-L2 cross-replica rate-limit accuracy** — carried since two phases ago.
   The new per-endpoint OAuth governor is in-process; multi-replica deployments
   still lack a shared rate-limit store. Still open.
4. **`introspection_delegate` body-size cap** — no explicit bound on the relayed
   Hydra response body (LOW). Same trust boundary as any upstream call.

## Lessons Captured (knowledge base)

- **"Audit every authorize entry point, not just the engine"** held again — the
  redirect-follow HIGH was in the *transport client*, not the exchange logic. A
  correct exchange with a redirect-following client is still an exfil vector.
  Reuse the project's own hardened pattern (`jwks.rs` `Policy::none()`) for any
  client that forwards a bearer credential.
- **A derived `Default` on a fail-closed config is a fail-open trap.** When the
  serde default and the `#[derive(Default)]` value disagree on a security flag,
  the derive silently wins in code paths that don't deserialize. Prefer an
  explicit `Default` that mirrors the serde default for any security toggle.
- **Federate-first is a real seam, not a slogan** — G4's delegate mode proves the
  "never an IdP" stance concretely: the gateway can hand the whole RFC 8693
  exchange to Hydra and relay its token, while still fail-closing locally on
  `actor_token` and transport failure.
- **Separated security review continues to earn its cost** — 1 HIGH + 3
  proactive hardenings across 4 changes, all fixed before archive, zero surviving
  to the commit.

## Recommended Next Phase

**`agent-gateway-exposure-operability`** — the OAuth/identity surface is now
*safe to expose*; the next phase makes exposure *operable and observable* at
multi-replica scale, plus the deferred edges this phase documented:

1. **Shared cross-replica rate-limit + introspection state (Redis-L2 as a real
   backend)** — resolve the carried-over debt so the per-endpoint OAuth governor
   and cache invalidation are accurate across replicas. *Do first — it gates
   horizontal exposure.*
2. **Delegate-mode observability + optional gateway re-stamp** — decide whether
   delegate-issued tokens should be re-classified/`flint_kind`-stamped for agent
   budget/rate-limit parity, or explicitly documented as escaping it; add metrics
   for delegate success/deny.
3. **Operator guardrails for the exposure surface** — `https`-only enforcement on
   `hydra_token_url`/`hydra_admin_url`, a body-size cap on relayed Hydra
   responses, and a startup posture check that refuses to expose `/oauth/*`
   without both `introspect_auth` and rate-limiting configured.
4. **End-to-end exposure smoke tests** — extend the docker-compose smoke stack +
   Playwright E2E to cover the authenticated `/oauth/token` + `/oauth/introspect`
   + Hydra-delegate paths against a real Ory stack.

Stay authorization-first; still avoid the LLM-ops bundle; still federate, never
become an IdP. This phase made the capabilities *safe to expose*; the next makes
that exposure *operable at scale and observable*.
