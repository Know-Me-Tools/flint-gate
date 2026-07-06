# Analysis — agent-gateway-exposure-operability

_Analyzed: 2026-07-06 · Mode: stack-specified (Rust/axum/tokio, Ory reference)_
_Research budget: within cap (Tier 1 gh + Tier 3 cargo registry + local source
inspection; Tier 2 docfork transient-failed, compensated with registry + known
pattern + local crate source)._

## Standing constraint (load-bearing)

Support any IdM with a JWKS pathway; **Ory Kratos/Hydra is the standard**;
**federate, never become an IdP.** This directly governs the G2 re-stamp
decision below.

## Landscape summary

Three of the four goals are **build-with-existing-dependencies**, not
library-adoption decisions — the crates are already in the tree:

- `RedisRateLimiter` + `redis 1` (G1) — already present, default feature.
- `governor 0.10` / `tower_governor 0.8` (in-process governor) — already present;
  `KeyExtractor` trait confirmed in local source
  (`tower_governor-0.8.0/src/governor.rs`), but **irrelevant to cross-replica**
  accuracy (see G1 verdict).
- `reqwest 0.12`, `axum 0.8`, `tracing 0.1` — the guardrail + posture work (G3)
  is pure gateway code against these.

Only **G2 (metrics)** is a genuine build-vs-adopt call requiring a new crate.

## Build-vs-adopt calls

### G1 — Cross-replica OAuth rate limiting · **BUILD (wire existing) — no new dep**

The shared limiter exists (`RedisRateLimiter::incr_request`, Lua-atomic
fixed-window, `#[allow(dead_code)]`). The correct fix is a thin axum
middleware/extractor on the `/oauth/*` router that calls `incr_request` keyed by
`client_id` (falling back to peer IP for the token endpoint's pre-auth surface),
returning `429` past the window.

**Why not `tower_governor` with a custom `KeyExtractor`?** `governor` is an
**in-process** quanta-clock limiter — no custom key makes it cross-replica. Using
it for the OAuth surface would re-introduce the exact per-replica inaccuracy this
phase exists to close. **Verdict: BUILD a small Redis-backed layer; do not adopt
a second rate-limit lib.** Keep the in-process governor only as the documented
degrade target (see posture below).

**Redis-outage posture (open Q1) — RECOMMEND config toggle, default fail-closed
for the introspection oracle / degrade-with-warn for the token endpoint.**
Rationale: `/oauth/introspect` is a token-scanning oracle (RFC 7662 §2.1 threat)
— losing its rate limit is a security regression, so default **deny** on Redis
outage. `/oauth/token` failing fully closed on a Redis blip is an availability
cliff; default **degrade to the in-process governor + WARN**. Expose one
`oauth.rate_limit.on_backend_unavailable: deny | degrade` knob so operators can
force uniform behavior. (Confirm in Spec.)

### G2 — Delegate observability metrics · **ADOPT `metrics` + `metrics-exporter-prometheus`**

| Candidate | Version | Verdict | Evidence / rationale |
| --- | --- | --- | --- |
| `metrics` (facade) + `metrics-exporter-prometheus` | 0.24 / 0.18 | **ADOPT** | Lightweight facade decouples `counter!`/`histogram!` call-sites from the exporter; `PrometheusBuilder::install_recorder()` → `handle.render()` served on a `/metrics` axum route. No async-runtime coupling. Actively maintained (Tier 3). Idiomatic for a control-plane counter surface. |
| `opentelemetry` + `_sdk` | 0.32 | reject (this phase) | Heavier; frequent breaking churn (0.x cadence) = maintenance cost; OTLP export is more than a delegate counter needs. Revisit if org-wide OTEL is mandated. |
| `axum-prometheus` | 0.10 | reject | Auto HTTP-latency middleware — solves per-route HTTP metrics, not *delegate-semantic* counters (success/deny/latency by reason). Wrong granularity; would still need the facade. |
| `prometheus` (tikv) | 0.14 | reject | Lower-level registry; more boilerplate than the `metrics` facade for the same result. |

Serve `/metrics` on the **admin port** (private) — not the public proxy surface —
consistent with keeping the control-plane observable but not exposed. Instrument
`token_exchange.rs` delegate paths: `delegate_total{result=success|deny,reason}`
+ a latency histogram.

### G2 re-stamp (open Q2) · **RECOMMEND: do NOT re-stamp; document + meter the gap**

Re-stamping a Hydra-minted delegated token with `flint_kind=agent` would mean the
gateway **rewrites a token another authority issued** — a step toward acting as
an IdP, which violates the standing "federate, never an IdP" constraint. Instead:
keep delegate-mode tokens as Hydra issued them, **document** that they are subject
to Hydra's own claims (not the gateway's agent-budget classification), and
**meter** delegate issuance so operators can see the volume that bypasses
gateway-side agent budgets. If budget parity is later required, the correct place
is a Hydra-side claim mapper, not gateway rewriting. (Decision recorded below.)

### G3 — Operator guardrails · **BUILD — no new dep; mirror `admin_auth_posture`**

- **https-only Hydra URLs (open Q4):** validate `hydra_token_url` /
  `hydra_admin_url` scheme at startup; reject `http://` unless
  `allow_insecure_upstream: false`-defaulted config field (loud, off by default,
  NOT an env-only footgun). Parse with `url::Url` (already transitively present
  via reqwest) or a `starts_with` guard.
- **Body-size cap:** replace unbounded `resp.json::<Value>()` with a
  `bytes()`-with-limit read (e.g. 64 KiB) then parse — closes the documented LOW.
- **`/oauth/*` refuse-start posture:** add `oauth_exposure_posture()` mirroring
  `admin_auth_posture()` (`config/types.rs:177`) — `RefuseStart` when `/oauth/*`
  would bind non-loopback without **both** `introspect_auth` and rate-limiting.

### G4 — E2E vs a real Ory stack · **ADOPT existing stack; extend, don't rebuild**

Reuse `docker-compose.smoke.yml` + `web/playwright.config.ts` +
`web/e2e/smoke.spec.ts` (already scaffolded). Add Ory Hydra (+ Kratos) services
to the smoke compose; add specs for authenticated `/oauth/token` +
`/oauth/introspect` + Hydra-delegate happy-path and the fail-closed denials. No
new test framework — Playwright is already the harness. Ory images are the
reference per the standing constraint.

## Open questions — resolved here (record in decision-log)

1. **Redis-outage posture** → config toggle; default deny for introspect, degrade+warn for token. *(RECOMMEND — confirm in Spec.)*
2. **Delegate re-stamp** → **NO** (federate-first); document + meter instead. *(DECIDED.)*
3. **Metrics backend** → `metrics` + `metrics-exporter-prometheus`, `/metrics` on admin port. *(DECIDED.)*
4. **Insecure-upstream override** → explicit config field, off by default, loud warn when enabled. *(RECOMMEND — confirm in Spec.)*

No contested stack choice (single stack, all calls have a clear winner) — no
elicitation required.

## Net effect on scope

One new dependency pair (`metrics` + `metrics-exporter-prometheus`). Everything
else is gateway code against crates already in the tree. Confidence: **high** on
G1/G3/G4 (local-source-confirmed), **high** on G2 adopt (registry + well-known
pattern; docfork was down but the wiring is stable and low-risk).
