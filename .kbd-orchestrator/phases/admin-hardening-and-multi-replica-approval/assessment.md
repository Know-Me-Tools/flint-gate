# Assessment — admin-hardening-and-multi-replica-approval

_Assessed 2026-07-08 against `goals.md`. Workspace green at entry (492 core
tests; prior three-phase approval flow committed). Traces: `config/types.rs`,
`admin/mod.rs`, `admin/auth.rs`, `approval/mod.rs`, `ratelimit/`, `main.rs`,
`config.example.yaml`._

---

## Headline: all four goals are NOT MET — work is well-scoped and additive

`tower_governor` and the `build_governor_layer` / `CredentialKeyExtractor`
infrastructure are **already in-tree** (imported in `Cargo.toml`, implemented
in `src/ratelimit/governor_layer.rs`, applied to the proxy and OAuth routers).
The admin router does not yet use it. This makes G1 (admin rate-limiting) a
**wiring change**, not a build change — the hardest part is already done.

The remaining three goals (G2 cap/janitor-config, G3 multi-replica, G4 auth
audit) are each well-bounded additive changes with no risky dependencies.

---

## G1 — Admin write-endpoint rate-limiting  ·  Verdict: NOT MET (HIGH-2)

**What exists:**
- `ratelimit::build_governor_layer(per_second, burst)` returns a ready-to-layer
  `GovernorLayer<CredentialKeyExtractor, NoOpMiddleware>`.
- `server.rate_limit: RateLimitConfig` applied to the proxy router in `main.rs`
  (lines 660-672).
- `oauth.rate_limit: RateLimitConfig` applied to the OAuth router in `main.rs`
  (lines 626-650).
- Admin router (`admin_router_with_auth`) has auth middleware but **zero
  rate-limit middleware**. All write endpoints (`POST /approvals/{id}/decision`,
  `POST /policies`, `POST /agent-identities`, `POST /routes`, `POST /api-keys`,
  `POST /signing-keys`, `POST /cache/invalidate`, all `DELETE /*`) are
  unrestricted.

**Gap:**
- No `AdminRateLimitConfig` or `server.admin_rate_limit` config block.
- `admin_router_with_auth` does not receive a governor layer.
- `main.rs` does not build or apply any rate-limit to the admin app.

**Implementation path (additive, reuses existing infra):**
1. Add `admin_rate_limit: Option<RateLimitConfig>` to `ServerConfig` (or a
   dedicated `AdminRateLimitConfig`). Sensible default: `None` (disabled by
   default on loopback-dev; must be opted in for production).
2. `admin_router_with_auth` accepts an `Option<GovernorLayer<...>>` and applies
   it to the protected sub-router (same pattern as proxy + oauth routers).
3. `main.rs` builds the layer from `initial_config.server.admin_rate_limit` and
   passes it to `admin_router_with_auth`.
4. `config.example.yaml` documents `server.admin_rate_limit`.
5. Tests: protected admin route returns 429 when limit exceeded; public probes
   (`/health`) bypass the limiter.

**Effort: LOW** — the rate-limit infra is already implemented; this is a
wiring + config change.

---

## G2 — `ApprovalManager` max-pending cap + janitor config  ·  Verdict: NOT MET (MEDIUM-1)

**What exists:**
- `ApprovalManager::register()` inserts into an unbounded `DashMap`. No cap.
- `janitor_interval` in `main.rs` is derived heuristically (TTL/2, min 10s,
  else 60s) but is not user-configurable.
- `ApprovalConfig` has only `enabled` and `ttl_seconds`.

**Gaps:**
- `ApprovalConfig` has no `max_pending: Option<usize>` field.
- `ApprovalManager::register()` never returns a cap error.
- `ApprovalError` has no `CapExceeded` variant.
- No `janitor_interval_seconds` in `ApprovalConfig`; the heuristic in `main.rs`
  is reasonable but not operator-overridable.

**Implementation path:**
1. Add `max_pending: Option<usize>` and `janitor_interval_seconds: Option<u64>`
   to `ApprovalConfig` (both serde `default`).
2. Add `ApprovalError::CapExceeded` variant.
3. `ApprovalManager::register()` checks `inner.len() >= max_pending` before
   inserting; on breach returns `Err(ApprovalError::CapExceeded)`.
4. In `middleware/pipeline.rs` and the processor, treat `CapExceeded` the same
   as `Err(...)` — fail-closed to Deny (emit deny event, do not panic).
5. `main.rs` reads `approval.janitor_interval_seconds` and uses it directly,
   falling back to the existing heuristic.
6. `config.example.yaml` documents both new fields.
7. Tests: register at cap returns CapExceeded; stream path on CapExceeded denies
   (not panics); janitor_interval_seconds is used when set.

**Effort: LOW-MEDIUM** — additive config fields + one new error variant with a
short propagation chain.

---

## G3 — Multi-replica approval routing  ·  Verdict: NOT MET (constraint documented, not resolved)

**What exists:**
- `config.example.yaml` documents the single-replica constraint:
  > "The pending-approval table is in-memory PER REPLICA (a decision must reach
  >  the replica holding the paused stream) — cross-replica routing is a
  >  follow-up."
- `README.md` `### Pending Approvals` section mentions the constraint.
- No startup warning when a multi-replica deployment is detected.
- No integration test that fails on a cross-replica decision.
- No sticky routing or shared-store implementation.

**Assessment: implement Option 3c (documented constraint + test + startup
warning).** Options 3a (sticky routing) and 3b (shared store) require a load
balancer integration (3a) or a Redis backing store design (3b). The gateway has
Redis (`cache.l2`) but `ApprovalManager` does not use it. A full shared-store
migration is out of scope this phase; the minimum viable fix is:
- A startup WARNING when `server.listen` binds a non-loopback address (i.e.,
  the deployment might be load-balanced) + `REPLICA_COUNT > 1` env var is set.
- An integration test that proves a decision routed to a different `ApprovalManager`
  instance returns `404 NotFound` (not a silent-allow), making the constraint
  machine-verifiable.
- A prominent README note pointing operators to sticky-session or service-mesh
  configuration.

**Effort: LOW** — startup warning + integration test; no new routing logic.

---

## G4 — Admin auth audit  ·  Verdict: PARTIAL GAP FOUND

**What exists (`admin/auth.rs` — well-implemented):**
- `require_admin_auth` middleware: fail-closed at every edge (`AuthError` →
  4xx/5xx); `Identity` inserted into extensions for attribution.
- SPA fallback is protected (proven by `spa_fallback_is_protected_when_auth_denies`
  test in `admin/auth.rs`).
- `/health` and `/ready` stay open (proven by test).
- JWT and Kratos session auth are both supported via the shared `Authenticator`
  trait — no separate admin identity model.
- `admin_auth_posture()` enforces: non-loopback + no auth → refuse-to-start.

**Gaps found:**
- **No CORS middleware on the admin router.** The admin UI (port 4457) is served
  from the same origin as the SPA static assets, so CORS is not required for
  the web UI itself. However, if an operator exposes the admin port via a reverse
  proxy with a different origin (a common pattern), cross-origin requests will
  fail silently without CORS headers. The admin router should either:
  (a) emit a startup warning when `admin_listen` is non-loopback and no CORS
  config is set, or (b) support an explicit `server.admin_cors` allow-list.
  **Severity: LOW** (only affects operators who intentionally expose the admin
  on a separate origin).
- **No token rotation or session expiry on admin JWTs.** The admin JWT is
  verified via the shared `Authenticator` (typically Kratos session or a bearer
  JWT). Token TTL/rotation is delegated to the upstream IdM (Ory Kratos or the
  JWT issuer). This is the correct design for a federated model (never an IdP).
  **No gap here** — token lifecycle is the IdM's responsibility.
- **No request-size cap on admin write endpoints.** A malformed or oversized
  request body (e.g., a very large Cedar policy upload) is not explicitly
  limited. Axum's default does not cap body size. **Severity: LOW-MEDIUM.**
  Add `axum::extract::DefaultBodyLimit::max(N)` to the protected router.

**Summary:** no CRITICAL/HIGH auth gaps. Two LOW/LOW-MEDIUM hardening items
(CORS warning + body size cap) worth addressing as part of this phase since
the admin router is already open for modification.

---

## Implementation plan summary

| Change | Goal(s) | Type | Estimated tasks |
|--------|---------|------|-----------------|
| `add-admin-write-rate-limiting` | G1 | Additive wiring | 5 |
| `add-approval-cap-and-janitor-config` | G2 | Additive config + error variant | 5 |
| `add-admin-hardening-and-multi-replica-doc` | G3 + G4 | Constraint test + startup warning + CORS warn + body limit | 5 |

**Total: 3 changes, 15 tasks.** No new external dependencies required — `tower_governor` is already in `Cargo.toml`.

---

## Constraints carried (verified — unchanged from prior phases)

- Admin server (4457) NEVER public — the new rate-limit config must not change
  this; `admin_listen` loopback enforcement stays.
- No secrets / signing keys committed.
- No broken existing tests; config priority CLI>env>YAML untouched (all new
  fields are additive with serde defaults).
- Federate any JWKS IdM (Ory reference), never an IdP.
- Approval flow: fail-closed at every edge (the cap must Deny, not hang).

---

## Open questions resolved

- **Multi-replica option:** Option 3c (documented constraint + test + startup
  warning). Option 3a/3b deferred — requires load-balancer integration or
  shared-store migration; out of scope this phase.
- **Rate-limit library:** `tower_governor` via `build_governor_layer` — already
  in-tree; no new dependency. Decision closed.
- **`max_pending` default:** 1000 (allows ~3 approvals per second at the
  default 300s TTL before filling; reasonable for a single operator surface).
- **Admin CORS:** Option (a) — startup warning when non-loopback bind + no CORS
  config; no full CORS middleware this phase (the admin UI co-locates, so CORS
  is only a concern for out-of-band API consumers).
- **Body size cap:** `DefaultBodyLimit::max(64 * 1024)` on the protected router
  (64 KiB; covers any reasonable Cedar policy or agent-identity payload).
