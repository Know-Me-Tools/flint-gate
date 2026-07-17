# Decision Log — agent-gateway-exposure-operability

### 2026-07-06 — G2 metrics backend
Options: metrics+metrics-exporter-prometheus vs opentelemetry vs axum-prometheus vs prometheus(tikv)
Decision: metrics 0.24 + metrics-exporter-prometheus 0.18 | Provenance: research (Tier 3 registry + known pattern)
Rationale: lightweight facade, right granularity for delegate counters, /metrics on admin port; OTEL churn/overkill rejected.

### 2026-07-06 — G2 delegate re-stamp
Options: re-stamp Hydra tokens flint_kind=agent vs document+meter the gap
Decision: DO NOT re-stamp; document + meter | Provenance: constraint (federate, never an IdP)
Rationale: rewriting another authority's token = IdP behavior; budget parity, if needed, belongs in a Hydra-side claim mapper.

### 2026-07-06 — G1 cross-replica limiter
Options: build Redis-backed /oauth/* layer (reuse RedisRateLimiter) vs tower_governor custom KeyExtractor
Decision: BUILD Redis-backed layer (reuse existing incr_request); no new rate-limit dep | Provenance: research (local source)
Rationale: governor is in-process/quanta-clock — no key makes it cross-replica; would re-introduce the per-replica inaccuracy this phase closes.

### 2026-07-06 — G1 Redis-outage posture (RECOMMEND, confirm in Spec)
Decision: config toggle oauth.rate_limit.on_backend_unavailable (deny|degrade); default deny for /oauth/introspect, degrade+warn for /oauth/token | Provenance: recommendation
Rationale: introspect is a token-scanning oracle (fail-closed); token endpoint availability cliff (degrade).
