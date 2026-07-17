# Decision Log — agent-gateway-budget-and-policy-operability

### 2026-07-07 — G1 lint severity default
Options: WARN-by-default + opt-in strict(refuse-start) vs refuse-by-default
Decision: WARN default + opt-in server.strict_agent_governance -> bail! | Provenance: in-repo precedent (admin_auth_posture/oauth_exposure_posture)
Rationale: the repo's posture gates allow the loose case by default and refuse only when actually exposed; a non-breaking default with a fail-safe escalation path matches that, and won't break existing loose configs on upgrade.

### 2026-07-07 — G1 agent-reachable route detection
Decision: a route is agent-reachable iff its RESOLVED provider (route.auth ?? site.default_auth) is Jwt or Mcp (JWKS-backed) | Provenance: research (local source, pipeline.rs:139-143)
Rationale: JWKS providers can carry an RFC 8693 act/agent token; Kratos=human, ApiKey=Service, Anonymous=none. Reuses the pipeline's own resolution — no new reachability model.

### 2026-07-07 — G4 fail-closed lifetime agent budget
Options: make the lifetime read return Unavailable vs refuse scope:agent+window:lifetime at config
Decision: refuse at config, folded into G1's lint | Provenance: recommendation
Rationale: threading outage state through the ledger path for a corner case is heavier; a config-lint refusal is one path, visible to the operator, consistent with the documented "fail-closed agent budgets need a fixed window."

### 2026-07-07 — G2 strict cross-replica rate-limit mode
Decision: new oauth.rate_limit.require_shared_backend (off by default) -> refuse-start when exposed non-loopback without a shared Redis limiter | Provenance: research (on_backend_unavailable only covers mid-request error)
Rationale: the existing posture covers Redis ERRORS, not "no shared limiter configured"; the strict knob makes cross-replica accuracy an enforced startup invariant.

### 2026-07-07 — G3 sugar shape + UI scope
Decision: config-block sugar compiling to Cedar (validated by the existing validator) THIS phase; admin-UI policy builder DEFERRED | Provenance: recommendation
Rationale: the config sugar delivers the ergonomics win with least surface; a UI builder is a separable larger effort.

### 2026-07-07 — External PDP / new deps
Decision: none — zero new dependencies | Provenance: research (local source)
Rationale: lint + metric + Cedar codegen all build on in-tree cedar-policy 4, metrics, the posture pattern, and the write-time validator.
