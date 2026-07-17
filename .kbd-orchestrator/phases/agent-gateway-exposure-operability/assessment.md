# Assessment — agent-gateway-exposure-operability

_Assessed: 2026-07-06 · Backend: openspec · against `goals.md` (4 goals)_
_Method: static inspection of the current tree at commit `0027efa`._

## Headline

The seeded scope **over-estimated G1**. A Redis-backed shared rate limiter and
L2 cache are **already implemented and default-on** — the real gap is a narrow
wiring miss (the OAuth endpoints still use the in-process governor) plus the
Redis-outage posture decision. G2 (delegate observability) and G3 (guardrails)
are genuine build gaps, but G3 has a strong existing template to mirror
(`admin_auth_posture`). G4 (E2E vs a real Ory stack) is real and mostly greenfield.

Net: this phase is **smaller than seeded on G1**, **as-seeded on G2/G3/G4**.

## Goal-by-goal gap analysis

### G1 — Shared cross-replica rate-limit + Redis-L2 · **MOSTLY BUILT — narrow wiring gap**

**Already present (evidence):**
- `crates/flint-gate-core/src/ratelimit/mod.rs` — `RedisRateLimiter` with a
  Lua-atomic `INCRBY`+`EXPIRE` fixed-window script; `redis-l2` is a **default**
  cargo feature (`Cargo.toml:58 default = ["redis-l2"]`). Live-Redis integration
  tests exist (`#[ignore]`d).
- `RedisRateLimiter::incr_request(scope, id, window)` — the shared **request-rate**
  counter the OAuth endpoints need — **exists but is `#[allow(dead_code)]`** (built,
  never called). This is the single most telling gap.
- `crates/flint-gate-core/src/cache/mod.rs` — `GateCache::connect_l2` +
  `l2_connection()`; `main.rs:282-289` connects L2 and constructs the limiter into
  `AppState.rate_limiter`.

**The gap (small, specific):**
1. `main.rs:459-482` builds the `/oauth/token` + `/oauth/introspect` router with
   the **in-process** `build_governor_layer(per_second, burst)` (line 469) — NOT
   the shared `RedisRateLimiter`. So multi-replica OAuth rate limiting is still
   per-replica-inaccurate. Route `/oauth/*` through `incr_request` (or a governor
   layer backed by it), keyed by client_id / peer.
2. **Redis-outage posture is undecided** — if the shared limiter's Redis is down,
   does the OAuth surface fail closed (deny), or degrade to the in-process
   governor with a logged warning? (Carried open question; security-vs-availability.)

**Estimated effort:** S–M (wire + posture + de-`dead_code` + tests). Much of the
seeded "build Redis-L2" work is already done.

### G2 — Delegate-mode observability + optional re-stamp · **REAL GAP**

- **No general metrics/Prometheus facility.** The only `metrics` in the tree are
  per-stream token counters (`middleware/pipeline.rs`, `stream/websocket.rs`) and
  DB usage_events — nothing for the OAuth/delegate control plane. No
  `counter!`/`histogram!`/`/metrics` endpoint.
- Delegate success/deny/latency are therefore unobservable. Need a metrics
  surface (Prometheus text endpoint on the admin port, or OTLP — open question)
  and delegate instrumentation in `token_exchange.rs`.
- The **re-stamp decision** (should Hydra-minted delegated tokens be
  re-classified `flint_kind=agent` for budget parity, or documented as escaping
  it) is a design call for Analyze — it touches the federate-first stance.

**Estimated effort:** M.

### G3 — Operator guardrails for the exposure surface · **REAL GAP — clear template**

- **https-only Hydra URLs:** none. `hydra_token_url`/`hydra_admin_url` are used
  as-given (no scheme check). Add a startup validation rejecting `http://` unless
  an explicit insecure-dev override.
- **Body-size cap on relayed Hydra responses:** none (`resp.json::<Value>()` in
  `token_exchange.rs` is unbounded — the documented LOW from last phase).
- **OAuth exposure startup posture:** **strong existing template** —
  `admin_auth_posture()` (`config/types.rs:177`) already returns
  `Enforce/AllowLoopback/RefuseStart` off loopback-bind detection, consumed at
  `main.rs:598-622`. Mirror it: refuse to start when `/oauth/*` is on a
  non-loopback bind without **both** `introspect_auth` **and** rate-limiting.

**Estimated effort:** M (three sub-items; posture mirrors an existing pattern).

### G4 — End-to-end exposure smoke tests vs a real Ory stack · **REAL GAP**

- `docker-compose.smoke.yml`, `docker-compose.yml`, `web/playwright.config.ts`,
  `web/e2e/smoke.spec.ts` all exist (scaffold from two phases ago).
- **No OAuth coverage** in any existing `.spec.ts`; only `docker-compose.yml`
  references Ory — the **smoke** stack does not yet stand up Hydra/Kratos.
- Need: add Ory (Hydra/Kratos) to the smoke stack; E2E specs for authenticated
  `/oauth/token` + `/oauth/introspect` + Hydra-delegate happy-path **and** the
  fail-closed denials (unauth, over-rate, Hydra error/redirect).

**Estimated effort:** M–L (real-stack orchestration is the cost driver).

## Cross-cutting observations

- **Reuse-first strongly favored on G1 and G3** — the limiter and the posture
  pattern already exist; this phase should *wire and mirror*, not rebuild. Guard
  against re-implementing what `ratelimit/mod.rs` and `admin_auth_posture` provide
  (the same "defined-but-unwired" trap G4 of last phase hit with the Hydra seam).
- **Fail-closed discipline continues** — each new exposure/guardrail path needs a
  `degrades_to_deny`-style test (Redis-outage posture, refuse-start posture,
  https-reject, over-rate deny).
- **Federate-first stance is load-bearing for the G2 re-stamp question** — resolve
  in Analyze before it becomes a spec.

## Open questions for Analyze/Spec

1. **Redis-outage posture** for the OAuth surface: fail-closed (deny) vs degrade
   to in-process governor + warn. (Security-vs-availability — likely a config
   toggle defaulting to the safer choice.)
2. **Delegate re-stamp**: re-classify Hydra-minted delegated tokens as Agent, or
   document as escaping gateway budget? (Federate-first tension.)
3. **Metrics backend**: Prometheus text endpoint on the admin port vs OTLP export
   — align with any existing observability wiring / deployment expectation.
4. **Insecure-dev override** shape for https-only enforcement (env flag vs config
   field) — must be loud and off-by-default.

## Recommended change ordering (for Plan)

1. **G1 wiring + Redis-outage posture** (BUILD FIRST — the horizontal-exposure
   gate; smallest real gap, unblocks accurate multi-replica exposure).
2. **G3 guardrails** (https-only + body-cap + refuse-start posture — enforce the
   documented caveats; mirrors `admin_auth_posture`).
3. **G2 delegate observability** (+ re-stamp decision from Analyze).
4. **G4 E2E vs real Ory stack** (proves 1–3 end-to-end; depends on them landing).

Success criteria in `goals.md` are still valid; G1's is now largely a
wiring/posture/test criterion rather than a from-scratch build.
