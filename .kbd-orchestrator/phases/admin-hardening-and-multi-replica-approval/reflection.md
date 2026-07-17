# Reflection — admin-hardening-and-multi-replica-approval

_Phase completed: 2026-07-08_
_Changes: 3 / 3 DONE · Tests: 502 passed, 0 failed (8 ignored)_

---

## Goal Achievement

| Goal | ID | Status | Notes |
|------|----|--------|-------|
| Admin write-endpoint rate-limiting | G1 | **MET** | `tower_governor`/`GovernorLayer` applied to all admin write routes; `/health`, `/ready`, `/metrics` bypass it; `429` on burst exhaustion; `admin_rate_limit` block live in `config.example.yaml` |
| `ApprovalManager` cap + janitor config | G2 | **MET** | `ApprovalError::CapExceeded` + fail-closed deny path; `max_pending` defaults `Some(1000)`; `janitor_interval_seconds` explicit config with fallback heuristic; wired end-to-end in `main.rs` |
| Multi-replica constraint: document + test | G3 | **MET** | `cross_replica_decision_returns_not_found` machine-verifies the per-replica isolation; startup `warn!()` on non-loopback bind with `REPLICA_COUNT` escalation; README multi-replica table with sticky-session guidance and Option 3a/3b roadmap reference |
| Admin auth audit (gap sweep) | G4 | **MET** | Body-limit `DefaultBodyLimit::max(64 KiB)` on the protected sub-router; startup CORS `warn!()` on non-loopback admin bind; no CRITICAL/HIGH auth gaps found beyond items already addressed by prior phases |

**Overall: 4 / 4 goals MET. Phase declared COMPLETE.**

---

## Delivered Changes

### G1 · `add-admin-write-rate-limiting`

- **`config/types.rs`**: `admin_rate_limit: Option<RateLimitConfig>` added to `ServerConfig` (`#[serde(default)]`).
- **`admin/mod.rs`**: `admin_router_with_auth` accepts `Option<AdminGovernorLayer>` and layers it onto the protected sub-router; public probes skip it. Type alias `AdminGovernorLayer` defined for readability.
- **`main.rs`**: builds the governor layer from `initial_config.server.admin_rate_limit` and threads it to `admin_router_with_auth`.
- **`config.example.yaml`**: `server.admin_rate_limit` block documented (`per_second: 10`, `burst: 20`, per-replica note).
- **Tests added (5):** `admin_rate_limiter_returns_429_after_burst_exhausted`, `health_probe_bypasses_rate_limiter`, `no_rate_limiter_leaves_routes_unrestricted`, `admin_body_over_64k_returns_413`, `admin_body_at_limit_passes_body_limit_layer`.

### G2 · `add-approval-cap-and-janitor-config`

- **`approval/mod.rs`**: `ApprovalError::CapExceeded` variant; `cap: Option<usize>` field; `ApprovalManager::with_cap(n)` constructor; cap check in `register()`.
- **`stream/ag_ui.rs`** + **`stream/a2ui.rs`**: `CapExceeded` WARN arm before the generic error catch-all — fail-closed deny path with distinguishable log level.
- **`config/types.rs`**: `ApprovalConfig::max_pending` (`Some(1000)` default via `default_approval_max_pending`) and `janitor_interval_seconds` (`None` default).
- **`main.rs`**: approval manager wired from cap config; janitor interval respects explicit config, falls back to `ttl/2, clamped [10,300], else 60`.
- **`config.example.yaml`**: `approval.max_pending` and `approval.janitor_interval_seconds` documented.
- **Tests added (4):** `cross_replica_decision_returns_not_found`, `register_at_cap_returns_cap_exceeded`, `second_register_when_under_cap_succeeds`, `pipeline_cap_exceeded_denies_not_panics`, `janitor_interval_config_is_used_when_set`.

### G3+G4 · `add-admin-hardening-and-multi-replica-doc`

- **`admin/mod.rs`**: `DefaultBodyLimit::max(64 * 1024)` applied to the protected sub-router (innermost layer; order: `body-limit ← rate-limit ← auth ← handler`).
- **`approval/mod.rs`**: `cross_replica_decision_returns_not_found` — two independent `ApprovalManager` instances; decision on the wrong replica returns `Err(ApprovalError::NotFound)`. This makes the per-replica constraint machine-verifiable.
- **`main.rs`**: startup `warn!()` when `admin_listen` is non-loopback (with `REPLICA_COUNT` env-var escalation); CORS `warn!()` when admin bind is non-loopback without explicit CORS config.
- **`README.md`**: `### Pending Approvals` expanded — multi-replica deployment table (sticky sessions vs. shared store), `REPLICA_COUNT` env var hint, Kubernetes manifest sketch, Option 3a/3b roadmap stub.

---

## Test Summary

| Suite | Pass | Fail | Ignored |
|-------|------|------|---------|
| `flint-gate-core` (main) | 502 | 0 | 8 |
| `flint-gate` (binary) | 5 | 0 | 0 |
| Other workspace crates | 32 | 0 | 0 |
| **Total** | **539** | **0** | **8** |

_(8 ignored tests are pre-existing integration tests requiring a live external service; unchanged this phase.)_

---

## Artifact Quality Summary

No artifact-refiner QA gate was configured for this phase (`.refiner/artifacts/` contains no entries for these changes). All changes passed the ad-hoc verification gate (compiler green, clippy clean, tests passing) per the change tasks.

| Metric | Value |
|--------|-------|
| Changes with automated QA | 0/3 (refiner not configured) |
| Build/clippy/test gate pass rate | 3/3 (100%) |
| Changes requiring re-work | 1* |

\* `add-admin-write-rate-limiting` required a constructor-lookup fix (`GateCache::from_config`, `Router::from_config`, `AuthzEngine::empty()`, `use super::{admin_router_with_auth, AdminState}`) and a non-exhaustive-match fix after adding `CapExceeded`; these were caught by the compiler and fixed within the same change.

---

## Technical Debt Introduced

| Item | Severity | Notes |
|------|----------|-------|
| CORS config field `server.admin_cors` absent | LOW | Startup WARN fires whenever admin bind is non-loopback; actual CORS middleware is future work. Documented. |
| Multi-replica option 3a/3b not implemented | LOW | Constraint is machine-tested and documented; sticky-session or shared-store path is explicit roadmap work (Option 3a/3b). Not a surprise gap. |
| Refiner not configured | LOW | No `.refiner/` constraint file exists for this phase-line; future phases should add one if the team wants formal artifact QA gates. |
| `DefaultBodyLimit` is a tower middleware global | INFO | The 64 KiB body limit applies to the protected admin sub-router only (not public probes); axum's per-route override is available if an endpoint needs a larger payload in future. |

---

## Lessons Captured

1. **Constructor discovery before test helpers.** Helper functions like `minimal_admin_state()` that call constructors from other modules need to resolve constructor signatures first (`cargo check` catches this, but `grep`-ing the module is faster). Verified constructors: `GateCache::from_config(&CacheConfig::default())`, `Router::from_config(&gate_config)`, `AuthzEngine::empty()`.

2. **Layer ordering matters for both semantics and test isolation.** `body-limit ← rate-limit ← auth ← handler` ensures: (a) auth rejects unauthenticated requests before consuming rate-limit quota, (b) body-size is enforced before handlers touch the body, (c) public probes bypass both security layers. Documenting this in a comment prevented confusion during test writing.

3. **Non-exhaustive enum matches are compiler-enforced but silently expected.** Adding `ApprovalError::CapExceeded` caused a compile error in the `decide_approval_handler` match — expected, but notable: every match site for `ApprovalError` must be audited when adding a variant. The `stream/ag_ui.rs` and `stream/a2ui.rs` sites needed a specific WARN arm; the admin handler needed a 503 arm (with a comment that `decide()` never actually returns this variant from the current logic path).

4. **Fail-closed via enum, not panic.** `CapExceeded` returns a structured error that propagates up to the deny path cleanly; the stream processor never sees a panic. This is the correct pattern for resource-pressure errors in a gateway.

5. **Variable scope across conditional blocks.** `admin_is_non_loopback` computed inside a `{ }` block is invisible to code after it. Hoist early or use a let-binding at function scope.

6. **Machine-verifiable constraints are better than prose guarantees.** The multi-replica isolation test (`cross_replica_decision_returns_not_found`) is a stronger guarantee than any README paragraph. If the `ApprovalManager` is ever changed to share state (e.g., Redis backend), this test will fail-fast — which is exactly what we want.

---

## Recommended Next Phase

**Option A: Cedar policy authoring UX + policy hot-reload hardening**

The Cedar engine is live with hot-reload and write-time validation (prior phases). Gaps remaining:
- No policy authoring workflow for operators beyond raw Cedar syntax editing.
- No test harness for policies (simulate agent requests, verify policy decisions).
- Hot-reload error recovery: if a policy file becomes invalid between reloads, the engine falls back to the last valid policy silently — operators need a visible error state.
- Admin UI policy editor is a stub; it should surface parse errors inline.

_Priority:_ Operator experience for Cedar now that the engine is production-hardened.

**Option B: Step-up authentication flows**

JWT step-up triggers (demand a stronger credential mid-session) were explicitly deferred in the `agent-approval-and-step-up-flows` phase. With approval routing documented and rate-limiting in place, step-up is the next trust-level primitive.

**Option C: Quorum / multi-approver approval policies**

Currently a single-decision HITL model. Multi-approver quorum is the next governance escalation step (e.g., require 2 of N operators to approve a high-risk tool call).

---

**Recommendation:** Option A (Cedar authoring UX) — it unblocks operators from using the policy engine in practice, and it is the smallest scope that delivers visible value from all the prior phase investment. Step-up and quorum follow naturally once operators can author and test policies.
