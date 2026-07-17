# Plan — beta-release-readiness

_Generated: 2026-07-09_
_Backend: openspec_
_Assessment: `.kbd-orchestrator/phases/beta-release-readiness/assessment.md`_

---

## Ordering Rationale

The 14 gaps from the assessment are ordered by: (1) safety before features —
blockers that cause silent correctness failures or security incidents go first;
(2) prerequisites before dependents — the Cedar schema change (B-4) must land
before any tests that validate schema-aware policies; (3) infrastructure before
docs — runtime correctness changes before operator runbooks that document the
correct behavior.

Changes within a phase can be applied in parallel by separate agents but are
listed in the recommended serial order for a single-agent run.

---

## Phase 1 — Blockers (apply first, in order)

These three gaps will cause incidents in any multi-replica or cloud deployment.
None can be deferred.

| Order | Change ID | Gap | Files affected |
|-------|-----------|-----|----------------|
| 1 | `fix-tls-silent-fallback` | B-3: TLS misconfiguration silently falls back to plaintext | `main.rs`, `config/types.rs`, `config.example.yaml` |
| 2 | `fix-admin-k8s-exposure` | B-2: Admin API reachable cluster-wide in k8s without auth | `k8s/service.yaml`, `k8s/deployment.yaml`, `main.rs`, `docs/getting-started.md` |
| 3 | `fix-cross-replica-approvals` | B-1: Approval decisions silently fail on wrong replica | `k8s/service.yaml`, `approval/mod.rs`, `main.rs`, `config.example.yaml`, `docs/` |

**Why TLS before admin k8s exposure:** B-3 is a single-file config change with
no dependencies. B-2 requires touching the k8s service, which must come after
B-3 is stable so we don't introduce two unrelated k8s manifest changes in one
commit. B-1 (approvals) is addressed last in Phase 1 because the k8s
sessionAffinity fix (B-2) must be reviewed before B-1's Postgres-backed store
can be scoped (we need to know whether the sticky approach is sufficient for
beta or whether a shared store is required).

---

## Phase 2 — Serious Gaps (apply in order after Phase 1)

These gaps compromise correctness or security but do not cause immediate
incidents in a minimal deployment.

| Order | Change ID | Gap | Files affected |
|-------|-----------|-----|----------------|
| 4 | `fix-websocket-tool-authz` | S-2: WebSocket path bypasses Cedar + approval entirely | `stream/websocket.rs`, `middleware/pipeline.rs` |
| 5 | `fix-strict-agent-governance-default` | S-3: Ungoverned agent routes silently pass | `main.rs`, `config/types.rs`, `config.example.yaml` |
| 6 | `add-cedar-entity-schema` | S-4: Cedar schema validation not enforced at write time | `authz/engine.rs`, `authz/bundle.rs`, `db/mod.rs`, `admin/mod.rs` |
| 7 | `add-schema-migrations` | S-1: No schema migration versioning | `db/mod.rs`, `migrations/*.sql` (new), `Cargo.toml` |
| 8 | `add-approval-expiry-integration-test` | S-5: Approval TTL expiry has no E2E test | `sdks/go/integration_test.go`, `sdks/typescript/src/__tests__/integration.test.ts`, `config.test.yaml` |

**Why websocket before schema:** The WS path gap (S-2) is the most serious
correctness issue — tool calls bypass all authorization. It shares no
prerequisites with Cedar schema work. Cedar schema (S-6) is ordered after
governance default (S-5) because once strict governance is on, Cedar schema
validation makes it much easier to write correct policies. Migrations (S-7)
land after Cedar schema because the schema change may require a new column
(`schema_json` population logic).

---

## Phase 3 — Medium/Low Gaps (apply in order after Phase 2)

These gaps are operational friction rather than correctness or security issues.

| Order | Change ID | Gap | Files affected |
|-------|-----------|-----|----------------|
| 9 | `fix-multi-replica-rate-limit-warning` | S-6: Rate limits are per-replica with no multi-replica warning | `main.rs` |
| 10 | `add-changelog` | S-7: No CHANGELOG or breaking-change tracking | `CHANGELOG.md` (new), `release.yml` |
| 11 | `audit-hot-reload-rollback` | S-8: Hot-reload partial-config rollback unverified | `config/loader.rs`, `main.rs` |
| 12 | `fix-flutter-sdk-release` | S-9: Flutter SDK stub published in release | `release.yml`, `docs/docs/sdks/flutter.md` |
| 13 | `fix-admin-ui-readonly-indicators` | S-10: Read-only admin UI pages have no indicator | `web/src/pages/AuthProviders.tsx`, `web/src/pages/Hooks.tsx`, `web/src/pages/Budgets.tsx` |
| 14 | `add-operator-runbook` | S-11: No operator runbook | `docs/docs/operations.md` (new), `docs/docs/cedar-policies.md` (new) |

---

## Change Details

### Change 1: `fix-tls-silent-fallback`

**Gap:** B-3 — TLS misconfiguration silently falls back to plaintext TCP with only a `warn!` log.

**Approach:**
- Add `tls.fail_open: bool` (default `false`) to `ServerConfig` in `config/types.rs`
- In `main.rs:727–752`, when cert loading fails and `fail_open: false`, call `anyhow::bail!()` instead of falling through to plain TCP
- When `fail_open: true`, retain current behavior but emit an explicit `WARN: TLS fail-open is enabled — misconfigurations will start in cleartext`
- Update `config.example.yaml` with the new option and a production warning comment

**Success criteria:**
- `cargo test --workspace` passes
- Starting with `tls.enabled: true` and a missing cert path exits non-zero when `fail_open: false`
- Starting with `tls.enabled: true` and a missing cert path succeeds with a WARN when `fail_open: true`

---

### Change 2: `fix-admin-k8s-exposure`

**Gap:** B-2 — K8s service exposes admin port cluster-wide; no NetworkPolicy restricts it.

**Approach:**
- Add `k8s/network-policy.yaml` (new): NetworkPolicy that denies all ingress to port 4457 except from pods with label `flint-gate-admin-allowed: "true"`
- In `k8s/service.yaml`: ensure admin port is NOT exposed in the service (admin should only be accessible via pod IP, not via service ClusterIP)
- In `main.rs`, extend the startup posture guard: when `KUBERNETES_SERVICE_HOST` env var is present (running in k8s) AND `admin_listen` is loopback AND `admin_auth` is not configured, emit a `WARN` with specific k8s network advice
- Update `docs/docs/getting-started.md` with a red-box warning about k8s admin port exposure and the required NetworkPolicy

**Success criteria:**
- `k8s/network-policy.yaml` is syntactically valid Kubernetes YAML
- `docs/docs/getting-started.md` contains explicit admin port security section
- `cargo test --workspace` passes

---

### Change 3: `fix-cross-replica-approvals`

**Gap:** B-1 — `ApprovalManager` is in-process per-replica; multi-replica deployments silently drop ~50% of approval decisions.

**Approach (K8s sticky sessions — minimum viable for beta):**
- Add `sessionAffinity: ClientIP` and `sessionAffinityConfig.clientIP.timeoutSeconds: 10800` to the admin port service in `k8s/service.yaml` (or a separate `k8s/service-admin.yaml` dedicated to the admin port)
- Add a startup `warn!` when `REPLICA_COUNT > 1` OR `KUBERNETES_SERVICE_HOST` is set, listing the sticky-session requirement explicitly and linking to docs
- Document the per-replica constraint prominently in `config.example.yaml` approval block and `docs/docs/getting-started.md`
- Do NOT implement a Postgres-backed shared store in this change (follow-up phase) — the sticky session approach is sufficient for a beta with a small number of replicas

**Success criteria:**
- `k8s/service.yaml` or `k8s/service-admin.yaml` contains `sessionAffinity: ClientIP`
- Startup warning fires in tests when `REPLICA_COUNT=2` env var is set
- `cargo test --workspace` passes

---

### Change 4: `fix-websocket-tool-authz`

**Gap:** S-2 — `ws_bridge` does not receive tool-authz or `ApprovalManager` context; WebSocket upstreams bypass Cedar entirely.

**Approach:**
- Add `tool_authz: Option<ToolAuthzContext>` and `approval_handle: Option<ApprovalHandle>` parameters to `ws_bridge` in `stream/websocket.rs`
- Wire these from `middleware/pipeline.rs` at the WS branch, using the same authz context construction as the SSE branch
- For the approval path: when a WS text frame contains a `TOOL_CALL_START` event and Cedar returns `RequireApproval`, the bridge must pause the frame, register with `ApprovalManager`, and resume or drop based on the decision (mirrors the SSE processor logic)
- Add unit tests for the WS tool-authz path in `stream/websocket.rs`

**Success criteria:**
- A WebSocket request through a route with `type: authorize` hook has the same Cedar evaluation as an SSE request
- `cargo test --workspace` passes

---

### Change 5: `fix-strict-agent-governance-default`

**Gap:** S-3 — `strict_agent_governance: false` means ungoverned agent-reachable routes are silently proxied.

**Approach:**
- Change `strict_agent_governance` default in `config/types.rs` to `true`
- Update `config.example.yaml` to comment that this is now the default and explain how to disable
- Verify all existing tests still pass (the test config sets routes with explicit `authorize` hooks, so they should be compliant)
- Add a unit test that confirms a non-loopback-bound proxy with an ungoverned agent-reachable route and `strict_agent_governance: true` fails to start (or rejects the route on hot-reload)

**Success criteria:**
- `strict_agent_governance` is `true` by default
- `cargo test --workspace` passes
- `config.test.yaml` and all test fixtures are compliant with strict mode

---

### Change 6: `add-cedar-entity-schema`

**Gap:** S-4 — Cedar schema validation is not enforced; policy typos like `@require_apporval` are accepted silently.

**Approach:**
- Define the gateway's Cedar entity schema as a string constant in `authz/engine.rs` (or a separate `authz/schema.rs`):
  - Entity types: `User`, `Agent`, `Service`, `Route`, `Action` with `"call_tool"` as the only valid action
  - Annotation: `@require_approval(String)` as a known annotation
- Populate `schema_json` in `PolicyRecord` when validating policies via the Admin API
- In `validate_policy_handler` and `create_policy_handler` / `update_policy_handler`, validate against the schema — reject policies that reference undefined types or actions
- Add unit tests covering a typo'd `@require_apporval` annotation being rejected at write time

**Success criteria:**
- `POST /policies` with `@require_apporval` (typo) returns 422
- `POST /policies` with correct `@require_approval` is accepted
- `cargo test --workspace` passes

---

### Change 7: `add-schema-migrations`

**Gap:** S-1 — The schema DDL is one inline raw SQL string with `IF NOT EXISTS` guards and no version tracking.

**Approach:**
- Add `sqlx-cli` to dev dependencies in `Cargo.toml`
- Create `crates/flint-gate-core/migrations/` directory with numbered `.sql` files extracted from `SCHEMA_SQL`:
  - `0001_initial_schema.sql` — all current `CREATE TABLE IF NOT EXISTS` DDL
  - `0002_api_keys_role_column.sql` — the existing `ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS` statements
- Change `db.migrate()` to use `sqlx::migrate!("../migrations")` macro
- Remove the inline `SCHEMA_SQL` constant
- Ensure `cargo sqlx prepare` generates the `.sqlx/` offline query cache for CI (no live DB required)

**Success criteria:**
- `cargo test --workspace` passes (db tests use in-test Postgres or skip when no `DATABASE_URL`)
- Migration files are idempotent on a fresh schema and a schema that already has the tables

---

### Change 8: `add-approval-expiry-integration-test`

**Gap:** S-5 — The approval TTL janitor has no E2E integration test; expiry path is unit-tested only.

**Approach:**
- Add a Go integration test `TestIntegration_ApprovalExpiry` in `sdks/go/integration_test.go`:
  - Create a Cedar policy with `@require_approval`
  - Send a stream request; wait for the approval to appear
  - Wait for the approval TTL to expire (requires either a test-specific short TTL or a mock clock)
  - Assert the stream terminates and `ListApprovals` returns an empty list
- Add the same test in TypeScript
- Add a `config.test.yaml` option `approval.ttl_seconds: 5` to enable a short TTL for integration tests without touching the production default
- The janitor interval in test config should be set to 1s so the test doesn't take 60s

**Success criteria:**
- The expiry test passes in the integration test stack
- `go test -tags integration` passes
- `pnpm test:integration` passes

---

### Change 9: `fix-multi-replica-rate-limit-warning`

**Gap:** S-6 — Rate limits are per-replica; a multi-replica deployment silently multiplies the effective ceiling.

**Approach:**
- In `main.rs`, after rate limit configuration is applied, if any rate limiter is enabled (`admin_rate_limit.enabled`, `oauth.rate_limit.enabled`, `server.rate_limit.enabled`) and no Redis shared backend is configured (`cache.l2.enabled: false`), and the deployment is detected as multi-replica (`REPLICA_COUNT > 1` or `KUBERNETES_SERVICE_HOST`), emit:
  `WARN: rate limiting is enabled but no shared Redis backend is configured — effective ceiling is N × configured limit per replica`
- No behavior change, only observability improvement

**Success criteria:**
- `cargo test --workspace` passes
- Running with `REPLICA_COUNT=3` and rate limiting enabled prints the warning

---

### Change 10: `add-changelog`

**Gap:** S-7 — No CHANGELOG; beta customers who pin versions have no way to know about breaking changes.

**Approach:**
- Create `CHANGELOG.md` at repo root with a `## [Unreleased]` section and a `## [0.1.0] - 2026-07-09` section documenting all current capabilities
- Update `release.yml` to require that `CHANGELOG.md` is updated (add a `check-changelog` step that greps for the new tag version in `CHANGELOG.md` and fails if not found)
- Follow Keep a Changelog format (keepachangelog.com)

**Success criteria:**
- `CHANGELOG.md` exists and documents the 0.1.0 baseline
- The release workflow step fails if a tag is pushed without a matching CHANGELOG entry

---

### Change 11: `audit-hot-reload-rollback`

**Gap:** S-8 — Config hot-reload on invalid YAML/route config: unclear whether partial state is applied.

**Approach:**
- Read `config/loader.rs` fully and trace the hot-reload code path in `main.rs`
- If the reload is atomic (all-or-nothing), add a unit test confirming the previous config is retained on parse failure
- If the reload is non-atomic, make it atomic: parse and validate the new config fully before swapping the `SharedConfig` `RwLock`
- Add a test: write a valid config, trigger reload, write an invalid config, trigger reload, assert the previous config is still served

**Success criteria:**
- Hot-reload with an invalid config retains the previous config (verified by test)
- `cargo test --workspace` passes

---

### Change 12: `fix-flutter-sdk-release`

**Gap:** S-9 — Flutter SDK is ~155 lines of stub code; the release workflow publishes it to pub.dev.

**Approach:**
- Remove the `flutter-sdk` job from `.github/workflows/release.yml`
- Update `docs/docs/sdks/flutter.md` to explicitly state: "Flutter SDK: planned — not yet released. Subscribe to [GitHub releases] for availability."
- Remove the Flutter SDK from the SDK index page `docs/docs/sdks/index.md` or mark it as "coming soon"

**Success criteria:**
- `release.yml` has no `flutter-sdk` job
- Flutter SDK docs are marked as unreleased

---

### Change 13: `fix-admin-ui-readonly-indicators`

**Gap:** S-10 — `AuthProviders.tsx`, `Hooks.tsx`, `Budgets.tsx` display data with no indicator that they are read-only.

**Approach:**
- Add a reusable `ReadOnlyBanner` component that renders: "This view reflects the loaded configuration file. To make changes, edit `config.yaml` and restart (or trigger a hot-reload)."
- Add `<ReadOnlyBanner />` at the top of `AuthProviders.tsx`, `Hooks.tsx`, and `Budgets.tsx`
- Style the banner as a low-prominence info callout (not an error) using the existing shadcn/ui component set

**Success criteria:**
- The three pages display a read-only banner
- `pnpm build` (web) passes

---

### Change 14: `add-operator-runbook`

**Gap:** S-11 — No operator runbook; beta customers cannot self-serve on common failure modes.

**Approach:**
- Create `docs/docs/operations.md` covering:
  - Common failure modes: DB connection failure, Cedar reload failure, upstream unreachable, TLS misconfiguration
  - Multi-replica deployment checklist (admin auth required, sticky sessions required, rate limit shared backend required)
  - Log levels and what to look for (Cedar policy reload events, approval janitor events)
  - Metrics reference (list all counter/histogram names from `metrics.rs`)
- Create `docs/docs/cedar-policies.md` covering:
  - Cedar entity model for flint-gate (User, Agent, Service, Route, Action)
  - Annotated policy examples (require_approval, tool-scoping, budget enforcement)
  - Common mistakes (typo in annotation, missing entity type, forbid vs. permit interaction)

**Success criteria:**
- Both docs pages exist with substantive content (>200 lines each)
- Docusaurus sidebar updated to include both pages

---

## Summary

| Phase | Changes | Estimated effort |
|-------|---------|-----------------|
| 1 — Blockers | 1, 2, 3 | ~3–4 days |
| 2 — Serious | 4, 5, 6, 7, 8 | ~8–10 days |
| 3 — Medium/Low | 9, 10, 11, 12, 13, 14 | ~5–7 days |
| **Total** | **14** | **~3–5 weeks** |

**First change to apply:** `fix-tls-silent-fallback`
