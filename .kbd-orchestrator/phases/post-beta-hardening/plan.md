# Plan — post-beta-hardening

_Generated: 2026-07-10_
_Assessment: assessment.md_
_Backend: OpenSpec (openspec/ directory detected)_

---

## Ordering Rationale

Changes are ordered from smallest-blast-radius to largest, with
documentation first (no risk of breaking tests or builds), CI hardening
second (must be green before adding more code under test), and the
Postgres approval store last (highest-risk structural change that
touches the Axum state, the DB layer, and the K8s manifests).

SDK changes run in parallel batches once the CI gate is confirmed solid:
a broken SDK retry loop cannot break the gateway itself.

Goal 3 (Admin UI policy editor) is substantially met. The two remaining
UX gaps (syntax highlighting, approval→policy linkage) are bundled into
one lightweight change after the Postgres store is merged, since the
linkage depends on approval IDs surviving replica restarts.

---

## Changes — Ordered

### Change 1 — `add-metrics-docs`

**Goal:** G-4 (Metrics and observability documentation)
**Severity:** MEDIUM
**Effort:** 1 change
**Blocking:** nothing

**What:**
- Create `docs/docs/metrics.md` — reference page for all 6 Prometheus
  metrics exposed on `GET /metrics` (admin port).
- Add a sample Grafana dashboard JSON at `grafana/flint-gate-dashboard.json`.
- Update `docs/sidebars.ts` to surface the new page under Operations.
- Update `docs/docs/operations.md` "Monitoring" section with a reference
  to `GET /metrics` and the new reference page.

**Acceptance criteria:**
- `docs/docs/metrics.md` exists and documents all 6 metrics with name,
  type, label set, and alert-worthy thresholds.
- `grafana/flint-gate-dashboard.json` is valid JSON and imports cleanly
  into Grafana 10+.
- Sidebar entry renders correctly (structural check via `node -e`).

---

### Change 2 — `fix-approval-expiry-ci`

**Goal:** G-2 (CI integration tests — narrow TTL fixture gap)
**Severity:** LOW
**Effort:** 1 change
**Blocking:** nothing

**What:**
- Verify that `docker-compose.test.yml` mounts `config.test.yaml` with
  `ttl_seconds: 5` and `janitor_interval_seconds: 1`. If not, add the
  mount and the env override.
- Verify that `sdks/typescript/src/__tests__/integration.test.ts`
  contains an approval expiry test equivalent to
  `TestIntegration_ApprovalExpiry` in the Go SDK. If not, add it.
- Ensure the TypeScript test timeout is ≤ 30s (enough headroom below
  the CI 120s limit).

**Acceptance criteria:**
- `docker-compose.test.yml` passes `FLINT_APPROVAL_TTL_SECONDS=5` (or
  equivalent config mount) to the gateway container.
- A TypeScript integration test registers an approval, waits > TTL, and
  asserts that `GET /api/approvals/:id/status` returns `denied`.
- `pnpm test:integration` passes locally with the short-TTL fixture.

---

### Change 3 — `add-approval-store-trait`

**Goal:** G-1 — step (a): trait abstraction
**Severity:** HIGH
**Effort:** 1 change (of 3 for G-1)
**Blocking:** Change 4

**What:**
- Extract an `ApprovalStore` trait from `ApprovalManager` in
  `crates/flint-gate-core/src/approval/mod.rs`.
- Trait surface: `register`, `decide`, `list`, `status`,
  `purge_expired`, `earliest_expiry` — identical to the current concrete
  methods.
- Rename the existing `DashMap`-backed struct to `MemoryApprovalStore`
  and implement `ApprovalStore` for it.
- Thread `Arc<dyn ApprovalStore>` through `AppState` instead of the
  concrete type.
- All existing unit tests must remain passing; only the concrete type
  reference in `AppState` changes.

**Acceptance criteria:**
- `cargo test --workspace` passes with zero regressions.
- `AppState.approval_manager` (or equivalent field) holds
  `Arc<dyn ApprovalStore + Send + Sync>`.
- `MemoryApprovalStore` implements `ApprovalStore` and passes the
  existing unit test suite (including `cross_replica_decision_returns_not_found`).

---

### Change 4 — `add-postgres-approval-store`

**Goal:** G-1 — step (b): Postgres implementation + migration
**Severity:** HIGH
**Effort:** 1 change (of 3 for G-1)
**Blocking:** Change 5; requires Change 3

**What:**
- Add a `PostgresApprovalStore` struct implementing `ApprovalStore`.
- Add `sqlx` migration: `migrations/XXXX_pending_approvals.sql` with
  `CREATE TABLE pending_approvals (id UUID PRIMARY KEY, agent_sub TEXT,
  tool_name TEXT, reason TEXT, expires_at TIMESTAMPTZ, decision TEXT,
  decided_at TIMESTAMPTZ)`.
- `PostgresApprovalStore::register` inserts a row; `decide` updates
  `decision` + `decided_at`; `list` / `status` query the table;
  `purge_expired` deletes rows past `expires_at`.
- Use `LISTEN`/`NOTIFY` (`pg_notify('approval_decided', id)`) to enable
  cross-replica wake-up for the streaming approval response.
- Existing `MemoryApprovalStore` must remain the default when
  `approval.backend` is not set (or set to `memory`).

**Acceptance criteria:**
- `cargo test --workspace` passes.
- Integration test `TestIntegration_ApprovalExpiry` passes against a
  `PostgresApprovalStore`-backed gateway (add a `postgres` variant of
  the docker-compose test fixture).
- An approval registered on one replica can be retrieved by its ID via
  the API (proves cross-replica correctness).

---

### Change 5 — `wire-postgres-approval-config`

**Goal:** G-1 — step (c): config wiring + K8s cleanup
**Severity:** HIGH
**Effort:** 1 change (of 3 for G-1)
**Blocking:** nothing; requires Change 4

**What:**
- Add `approval.backend: postgres | memory` to `config.yaml` schema and
  the configuration loading path. Default: `memory`.
- When `approval.backend = postgres`, instantiate `PostgresApprovalStore`
  using the existing `DATABASE_URL` / `db.*` config.
- Update `docs/docs/operations.md` with the new config key and migration
  instructions.
- Remove the `sessionAffinity: ClientIP` band-aid from
  `k8s/service-admin.yaml` (or annotate it as no longer required when
  Postgres backend is in use).
- Update `config.test.yaml` to use `approval.backend: memory` explicitly
  so the existing short-TTL integration tests remain fast.

**Acceptance criteria:**
- `cargo test --workspace` passes.
- Setting `FLINT_APPROVAL_BACKEND=postgres` (or `approval.backend:
  postgres` in YAML) starts the gateway with the Postgres store.
- Setting neither (or `memory`) uses `MemoryApprovalStore` — existing
  behavior unchanged.
- `config_priority_order` tests still pass (CLI > env > YAML).

---

### Change 6 — `harden-go-sdk`

**Goal:** G-5 (Agent SDK enhancements — Go)
**Severity:** MEDIUM
**Effort:** 1 change
**Blocking:** nothing

**What:**
- Add a `TokenSource` interface to `sdks/go/` with a
  `GetToken(ctx) (string, error)` method. Default implementation wraps a
  static token string (backwards-compatible).
- Add `doJSON` retry-on-429 with exponential backoff (max 3 retries,
  initial 500ms, factor 2, jitter ±20%).
- Add `IsRateLimited(err error) bool`, `IsUnauthorized(err error) bool`,
  `IsApprovalRequired(err error) bool` helpers.
- Add SSE reconnect in `StreamSSE`: on `io.EOF` or network error,
  re-establish the connection with exponential backoff (max 5 retries).
- All new behaviour must be covered by unit tests.

**Acceptance criteria:**
- `go test ./...` passes in `sdks/go/`.
- A test demonstrates that a 429 response triggers retry and eventually
  succeeds (mock HTTP server).
- `IsRateLimited`, `IsUnauthorized`, `IsApprovalRequired` return correct
  values for the corresponding status codes.
- A test demonstrates that SSE reconnects after a simulated connection
  drop.

---

### Change 7 — `harden-ts-sdk`

**Goal:** G-5 (Agent SDK enhancements — TypeScript)
**Severity:** MEDIUM
**Effort:** 1 change
**Blocking:** nothing

**What:**
- Mirror the Go SDK hardening in `sdks/typescript/src/`:
  - `TokenProvider` type: `() => Promise<string>` (backwards-compatible
    with static string via adapter).
  - `doJSON` retry-on-429 with identical backoff parameters.
  - `isRateLimited(err)`, `isUnauthorized(err)`, `isApprovalRequired(err)`
    helpers.
  - `streamSSE` reconnect with exponential backoff.
- All new behaviour covered by unit tests (`vitest` or existing runner).

**Acceptance criteria:**
- `pnpm test` passes in `sdks/typescript/`.
- Same behavioural assertions as the Go SDK: 429 retry, error helpers,
  SSE reconnect.

---

### Change 8 — `add-policy-editor-ux`

**Goal:** G-3 (Admin UI policy editor — remaining UX gaps)
**Severity:** LOW
**Effort:** 1 change
**Blocking:** Change 5 (approval→policy linkage requires stable approval IDs)

**What:**
- Integrate CodeMirror 6 (or Monaco) into `web/src/pages/Policies.tsx`
  as a Cedar-aware editor:
  - Basic syntax highlighting for Cedar keywords (`permit`, `forbid`,
    `when`, `unless`, `principal`, `action`, `resource`).
  - Auto-indent on newline.
  - Replace the plain `<Textarea>` in `PolicyForm` with the editor.
- Add an "Approvals" chip/badge to each policy card in the policy list.
  When a policy has `@require_approval`, show the count of pending
  approvals and link to `Approvals.tsx` pre-filtered to that policy.

**Acceptance criteria:**
- The policy textarea renders with syntax highlighting.
- Existing inline validation (debounce + error list) continues to work
  in the new editor.
- A policy with `@require_approval` shows an "N pending" badge; clicking
  navigates to `/approvals?policy=<id>`.
- `pnpm typecheck` passes.

---

## Parallel Batches

Changes 1 and 2 can run concurrently (independent, no shared files).
Changes 6 and 7 can run concurrently (independent SDKs).
Changes 3 → 4 → 5 are sequential (trait → impl → config wiring).
Change 8 depends on Change 5 for the approval linkage; can start earlier
if the linkage feature is deferred.

```
Batch A (parallel): change-1 (add-metrics-docs)
                    change-2 (fix-approval-expiry-ci)

Batch B (sequential): change-3 (add-approval-store-trait)
                    → change-4 (add-postgres-approval-store)
                    → change-5 (wire-postgres-approval-config)

Batch C (parallel): change-6 (harden-go-sdk)
                    change-7 (harden-ts-sdk)

Batch D (after B): change-8 (add-policy-editor-ux)
```

Recommended execution order (single-agent sequential):
1, 2, 3, 4, 5, 6, 7, 8

---

## Change Summary Table

| # | Change ID | Goal | Severity | Depends On |
|---|-----------|------|----------|------------|
| 1 | `add-metrics-docs` | G-4 | MEDIUM | — |
| 2 | `fix-approval-expiry-ci` | G-2 | LOW | — |
| 3 | `add-approval-store-trait` | G-1a | HIGH | — |
| 4 | `add-postgres-approval-store` | G-1b | HIGH | 3 |
| 5 | `wire-postgres-approval-config` | G-1c | HIGH | 4 |
| 6 | `harden-go-sdk` | G-5 | MEDIUM | — |
| 7 | `harden-ts-sdk` | G-5 | MEDIUM | — |
| 8 | `add-policy-editor-ux` | G-3 | LOW | 5 |

**Total changes: 8**
**First change to apply: `add-metrics-docs`**
