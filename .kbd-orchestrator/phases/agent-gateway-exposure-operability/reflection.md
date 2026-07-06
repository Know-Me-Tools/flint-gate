# Reflection — agent-gateway-exposure-operability

_Phase closed: 2026-07-06 · Backend: openspec · Driver: kbd-apply_
_Seeded from: `agent-gateway-hardening-and-exposure/reflection.md`_

## Phase Goal (restated)

Take the now-safe-to-expose OAuth/identity surface and make it **operable at
horizontal scale and observable in production**: shared cross-replica rate
limiting, delegate-mode observability, enforced operator guardrails, and E2E
proof against a real Ory stack. Still authorization-first; still **federate any
JWKS-capable IdM (Ory reference), never an IdP**; LLM-ops bundle out of scope.

## Goal Achievement

| Goal | Status | Evidence |
| --- | --- | --- |
| **G1 — Shared cross-replica rate-limit** | ✅ MET | `add-oauth-shared-ratelimit`: `/oauth/*` now routes through the shared `RedisRateLimiter::incr_request` (authoritative across replicas), keyed by authenticated `client_id`; `oauth.rate_limit.on_backend_unavailable` posture (deny for the introspect oracle, degrade+WARN for token). The Assess finding that the limiter was **already built but unwired** (`incr_request` was `#[allow(dead_code)]`) meant this was a wiring+posture change, not a from-scratch build. |
| **G2 — Delegate observability + re-stamp decision** | ✅ MET | `add-delegate-observability`: adopted `metrics` + `metrics-exporter-prometheus`; `flint_delegate_total{result}` + latency histogram on every delegate outcome; `/metrics` on the **admin** port only. Re-stamp decision **DECIDED = NO** (federate-never-an-IdP) — documented + metered instead. |
| **G3 — Operator guardrails** | ✅ MET | `add-exposure-guardrails`: https-only Hydra URL validation (off-by-default `allow_insecure_upstream`); 64 KiB `read_capped_json` body cap (streaming abort); `oauth_exposure_posture()` RefuseStart mirroring `admin_auth_posture()`. |
| **G4 — E2E vs a real Ory stack** | ✅ MET | `add-oauth-e2e-ory`: Ory Hydra added to the smoke stack + `config.smoke.yaml`; `oauth.spec.ts` covers happy-path (RFC 8693 delegate exchange, authenticated introspect) + fail-closed denials (401/400/429), deterministic + opt-in. |

**4/4 goals MET (100%).** Every draft success-criterion in `goals.md` is
satisfied and test-proven (unit + real-stack E2E).

## Delivered Changes

| # | Change | Goal | Tasks | Status |
| --- | --- | --- | --- | --- |
| 1 | `add-oauth-shared-ratelimit` | G1 | 5/5 | archived |
| 2 | `add-exposure-guardrails` | G3 | 5/5 | archived |
| 3 | `add-delegate-observability` | G2 | 5/5 | archived |
| 4 | `add-oauth-e2e-ory` | G4 | 4/4 | archived |

Build order followed the plan (G1 gate → G3 guardrails → G2 observability → G4
E2E). Verification gate met per code change: `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo test --workspace` green (378 → **406**
core tests across the phase, +28 net new). One new dependency pair (metrics +
exporter), in change 3 only.

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA | 4/4 (100%) |
| First-pass pass rate | 4/4 (100%) — all PASS, none BLOCKED |
| Changes requiring refinement iteration | 0 (all review fixes applied inline, pre-archive) |
| Blocking-constraint violations | 0 across all 4 changes |
| Security findings surviving to archive | 0 |

### Blocking constraints — all PASS every change

`no-secrets`, `admin-4457-not-public`, `no-broken-tests`, `config-priority
CLI>env>YAML` passed as BLOCK-level in all four logs. No recurring violation
pattern — nothing failed even once.

### Security findings caught & remediated *before* archive

Separated security review (author never grades its own fail-open seam) ran on the
three code changes (G4 was compose/docs — a targeted self-check instead):

- **G1 — HIGH: header-rotation rate-limit bypass.** The limiter keyed on the raw
  Authorization header, but client-credentials arrive in the form body — an
  attacker could rotate the header to mint a fresh window while brute-forcing
  `client_secret`. Fixed: key on the authenticated `client_id`. Plus MEDIUM
  (anon-bucket, mitigated by the same fix) + LOW (`per_second==0` lockout guard).
- **G3 — MEDIUM: introspection-delegate redirect-SSRF.** The introspection
  delegate reused the redirect-following shared client while the token-exchange
  delegate used `Policy::none()` — a compromised Hydra 3xx could re-POST the
  introspected token. Fixed to parity (dedicated no-redirect client). Plus LOW
  (`saturating_add` on the body-cap check). All three exposure invariants
  otherwise held (loopback-detector stress-tested against crafted vectors).
- **G2 — APPROVE, no CRITICAL/HIGH.** `/metrics` admin-only (verified
  structurally); `record_delegate(&'static str)` structurally forbids an
  attacker value as a metric label.

## Technical Debt Introduced

1. **Redis-outage posture is a startup guarantee, not a runtime one.** The
   exposure posture requires `rate_limit.enabled` at startup, but the token
   endpoint can `degrade` to the in-process (per-replica) governor under a live
   Redis outage — so "rate-limited at startup" ≠ "cross-replica limited at
   runtime." Documented; an operator wanting strict cross-replica guarantees must
   set `on_backend_unavailable: deny` for the token endpoint too.
2. **Local-mint exchange path is not metered** — only the delegate path emits
   `flint_delegate_*`. Intentional (delegate observability), but there is no
   symmetric metric for locally-minted delegated tokens.
3. **Hydra-outage deny paths are unit-tested, not E2E-tested.** Reproducing a
   live Hydra 3xx/transport failure in the compose stack would be flaky; those
   remain covered by the deterministic Rust `delegate_fails_closed_on_*` tests.
4. **OAuth rate-limit anon bucket** — fully-unauthenticated callers still share a
   single `anon` window per endpoint; the per-replica IP governor is the shield
   there (the handler layer has no peer IP). Low impact (introspect requires auth;
   token callers present a credential).

## Lessons Captured (knowledge base)

- **"Already built but unwired" is a distinct Assess outcome worth naming.** G1's
  shared limiter existed with a `#[allow(dead_code)]` method and a default-on
  feature; the Assess correctly reframed the goal from "build Redis-L2" to "wire
  the OAuth endpoints + decide the outage posture." Grepping for `dead_code` /
  unused public API during Assess surfaces this cheaply. Same trap the *previous*
  phase hit with the Hydra-delegate seam.
- **Bring sibling code paths to security parity.** The G3 review's one real
  finding was the introspection delegate lacking the `Policy::none()` the
  token-exchange delegate already had — a copy that drifted. When one path is
  hardened against a class (redirect-SSRF, unbounded body), audit its siblings in
  the same review.
- **Rate-limit keys must bind the credential the endpoint actually authenticates**
  — not a convenient transport artifact (the raw header). Keying on the form/Basic
  `client_id` is what makes the limit a real brute-force control.
- **`&'static str` label types are a cheap cardinality/injection guardrail** — the
  metric API structurally refuses a runtime string, so no token/credential can
  ever become an unbounded label. Encode the safety in the type, not a review note.
- **Separated security review kept paying out** — 1 HIGH + 1 MEDIUM + several LOWs
  across 3 code changes, all fixed before archive, zero surviving to commit.

## Recommended Next Phase

**`agent-gateway-mcp-tool-governance`** — the identity/authz/exposure surface is
now safe, operable, and observable; the next frontier is the **MCP-era agent
gateway** value the project set out to build — governing *what tools an agent may
call and how much it may spend*, using the now-solid identity foundation:

1. **Per-tool-call authorization polish + Cedar policy ergonomics** — build on the
   existing embedded Cedar engine + per-tool-call authz to make agent tool-scoping
   first-class (tool allow/deny lists per agent identity, resource-scoped grants),
   with the delegate-classification gap from this phase resolved (a Hydra-side
   claim mapper, or an explicit gateway policy for delegate-mode tokens). *Do
   first — it's the core agent-gateway differentiator.*
2. **Agent budget enforcement at runtime** — wire the windowed token budgets to
   the *shared* Redis counters (now proven for rate-limiting) so agent spend is
   cross-replica-accurate, and close the delegate-token budget-bypass documented
   here (debt #1/#2).
3. **MCP tool-call observability** — extend the `metrics` surface (established this
   phase) to per-tool authz decisions + budget consumption, so operators see agent
   behavior, not just delegate volume.
4. **Optionally: the deferred operability edge** — a strict cross-replica
   rate-limit mode (token-endpoint `deny` posture) + a symmetric metric for the
   local-mint exchange path.

Stay authorization-first; still federate any JWKS IdM, never an IdP; the LLM-ops
bundle (semantic caching, routing/LB, prompt compression) remains out of scope
until the tool-governance core is complete. This phase made exposure operable;
the next makes the gateway **agent-aware** — the reason the project exists.
