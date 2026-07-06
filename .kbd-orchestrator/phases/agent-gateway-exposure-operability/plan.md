# Plan — agent-gateway-exposure-operability

_Planned: 2026-07-06 · Backend: openspec · Driver: kbd-apply (one task/turn)_
_Inputs: assessment.md, analysis.md, library-candidates.json, 4 OpenSpec specs._

## Ordering rationale (dependency-aware, security-gated)

The order follows the assessment's recommendation and the natural dependency
chain: the exposure **gate** (shared rate-limit) first, then the **enforced
invariants** (guardrails), then **observability**, then the **E2E** that
exercises all three against a real Ory stack. Changes 1–3 are independent of each
other (different files, no shared new types beyond config) but 4 depends on all of
them being present to test the real surface.

## Ordered changes

### 1. `add-oauth-shared-ratelimit` — G1 · **BUILD FIRST (the horizontal-exposure gate)**
- **Reuses:** `flint-gate-core::ratelimit::RedisRateLimiter::incr_request` (build-001 — do NOT adopt a second rate-limit crate; `tower_governor` is in-process only).
- **Delivers:** `/oauth/*` keyed by `client_id` through the shared Redis limiter; `oauth.rate_limit.on_backend_unavailable = deny | degrade` posture (default: deny for introspect, degrade+WARN for token).
- **Recommended agent:** rust-reviewer + security-reviewer (rate-limit bypass + outage-posture fail-open are the risks).
- **Gate:** `cargo check/clippy -D warnings/test --workspace`; deny-path tests (over-window 429; Redis-down deny/degrade); ≥80% new-code coverage.
- **Tasks:** 5.

### 2. `add-exposure-guardrails` — G3 · mirror `admin_auth_posture`
- **Reuses:** `config/types.rs::admin_auth_posture` pattern (build-002 — no new dep).
- **Delivers:** https-only Hydra URL validation (off-by-default `allow_insecure_upstream`); 64 KiB Hydra-response body cap; `oauth_exposure_posture()` RefuseStart on non-loopback `/oauth/*` without introspect_auth + rate-limit.
- **Recommended agent:** security-reviewer (each item is a fail-safe invariant; refuse-start posture must not fail open).
- **Gate:** refuse-start / http-reject / over-cap deny tests; workspace green; ≥80%.
- **Tasks:** 5.

### 3. `add-delegate-observability` — G2 · **ADOPT `cand-001`**
- **Library:** `cand-001` — `metrics@0.24` + `metrics-exporter-prometheus@0.18` (adopt; the ONLY new deps this phase). `/metrics` on the **admin** port.
- **Delivers:** `flint_delegate_total{result,reason}` + latency histogram on the delegate paths; encodes build-003 **no-re-stamp** decision (document + meter, per federate-never-an-IdP).
- **Recommended agent:** rust-reviewer (dep wiring + recorder lifecycle); security-reviewer light-touch (confirm `/metrics` is admin-port-only, no token bytes in labels).
- **Gate:** `/metrics` renders delegate counters; admin-port-only assertion; workspace green; ≥80%.
- **Tasks:** 5.

### 4. `add-oauth-e2e-ory` — G4 · **depends on 1–3**
- **Reuses:** existing smoke/Playwright harness (build-004 — extend, don't rebuild). Ory Hydra/Kratos = the standing reference.
- **Delivers:** Ory in `docker-compose.smoke.yml`; E2E for authenticated token/introspect/delegate happy-path + fail-closed denials (unauth 401, over-rate 429, Hydra error/redirect deny, actor_token 400).
- **Recommended agent:** e2e-runner (Playwright specs + compose orchestration).
- **Gate:** suite passes against the composed Ory stack; deterministic waits; documented CI entrypoint.
- **Tasks:** 4.

## Cross-cutting guardrails (apply to every change)

- **Fail-closed discipline** — each change carries a deny-path/refuse-start test;
  the code author never grades its own fail-open seam (separated security review).
- **Reuse-first** — changes 1, 2, 4 reuse existing code/patterns; only change 3
  adds dependencies. Guard against re-implementing the limiter or the posture.
- **Standing constraint** — federate any JWKS IdM, Ory reference, never an IdP
  (load-bearing in change 3's no-re-stamp decision).
- **Blocking constraints** (project) — no secrets committed; admin port 4457 not
  public (change 3's `/metrics` is admin-port = compliant); no broken tests;
  config priority CLI>env>YAML unchanged.

## Spec confirmations carried into Execute

- Change 1: finalize the outage-posture default per endpoint (deny introspect /
  degrade token) — spec'd; confirm the config shape at implementation.
- Change 2: `allow_insecure_upstream` as an explicit off-by-default field (not
  env-only); 64 KiB cap value.

## Execution

Backend openspec, driven by `/kbd-apply` one task per turn. Archive each change
via `openspec archive <id> --skip-specs --yes` (phase convention — no `specs/`
delta). Per-change QA: artifact-refiner + security-review before archive.

**First change to apply:** `add-oauth-shared-ratelimit`.
