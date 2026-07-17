# Assessment — post-beta-hardening

_Generated: 2026-07-10_
_Sycophancy correction: applied — findings anchored to observed codebase state_

---

## Verdict

**Three of five goals are fully achievable with incremental changes; two require
significant new work.** The beta-release-readiness phase left the codebase
defensible, and the operational surface is now cleaner than it was. The critical
remaining correctness gap (cross-replica approval) has a workaround in place but
not a durable fix. Goals 1, 2, and 4 are straightforward. Goals 3 and 5 require
the most implementation effort.

---

## Sycophancy-Correction Audit

Patterns suppressed before writing findings:

| S-Code | Pattern | Where it could surface |
|--------|---------|----------------------|
| S-01 | Approval-seeking (praising beta-readiness work) | Prior phase achievements irrelevant to this assessment |
| S-03 | Scope expansion (listing everything as a gap) | Only goals from goals.md are in scope |
| S-05 | Severity minimisation ("minor enhancement") | SDK gaps are real friction in production agent frameworks |
| S-07 | False completeness ("CI is wired") | integration.yml is wired — but the approval expiry test's coverage gap must be stated honestly |

---

## Goal 1 — Postgres-backed shared approval store

**Status: NOT MET — workaround in place, durable fix absent**

### Current state

`crates/flint-gate-core/src/approval/mod.rs` is a `DashMap`-backed in-process
store. The `ApprovalManager` struct has an explicit unit test
(`cross_replica_decision_returns_not_found`) that documents and proves the
per-replica isolation: an approval registered on replica-A is invisible to
replica-B. The sticky-session workaround (`k8s/service-admin.yaml`) from
the previous phase is in place.

### What is missing

No Postgres-backed approval store exists anywhere in the codebase. There is no
`authz_pending_approvals` table, no DB query in `db/mod.rs`, and no migration
for such a table. The `ApprovalManager` trait boundary is not abstracted —
the concrete `DashMap` struct is used directly throughout. Making it
swappable requires:

1. A trait extraction (`ApprovalStore` or similar) with the same `register` /
   `decide` / `list` / `status` / `purge_expired` surface
2. A `PostgresApprovalStore` implementation persisting to a new `pending_approvals`
   table with Postgres `NOTIFY` for cross-replica wake-up
3. A `sqlx` migration for the `pending_approvals` table
4. Wiring the concrete implementation via config (`approval.backend: postgres | memory`)

The sticky-session band-aid survives a rolling deploy but not a pod crash.
A Postgres-backed store would survive both.

**Effort estimate:** 3–4 changes (trait + Postgres impl + migration + config wiring)

---

## Goal 2 — CI integration test wiring

**Status: PARTIALLY MET — wired for push/PR; approval expiry test not confirmed
in the fixture**

### Current state

`.github/workflows/integration.yml` is fully wired:
- Triggers on push and PR to `main` and `claude/flint-gate-auth-proxy-zQBD4`
- Starts `docker-compose.test.yml` before tests
- Runs `go test -v -tags integration -timeout 120s ./...` in `sdks/go`
- Runs `pnpm test:integration` in `sdks/typescript`
- Tears down after all tests

`TestIntegration_ApprovalExpiry` exists in `sdks/go/integration_test.go` (line
371) and is tagged `integration`. It will run in CI via the existing workflow.

### What is missing

The TypeScript counterpart of `TestIntegration_ApprovalExpiry` was specified in
the plan but needs verification in `sdks/typescript/src/__tests__/integration.test.ts`.
Additionally, there is no evidence that `docker-compose.test.yml` uses
`config.test.yaml` with `ttl_seconds: 5` rather than the default 300s TTL.
If the fixture uses the default TTL, the approval expiry test will either time
out (CI has a 120s Go test timeout) or be skipped.

**Effort estimate:** 1 change to verify/fix the TypeScript approval expiry test
and confirm `docker-compose.test.yml` uses the short-TTL test config.

---

## Goal 3 — Admin UI Cedar policy editor

**Status: SUBSTANTIALLY MET — more complete than expected**

### Current state

`web/src/pages/Policies.tsx` is a full CRUD surface for Cedar policies. It
includes:

- **Inline validation**: `validatePolicy()` API call with 500ms debounce on
  every keystroke; inline error list with line/column positions from
  `PolicyParseError`
- **Create/edit modal**: `PolicyForm` with a textarea for Cedar text, schema
  JSON, entities JSON, and enabled toggle
- **Delete**: confirmed via `window.confirm` with toast feedback
- **Version history**: `PolicyVersionHistory` component with paginated version
  list, text/diff view toggle, "Restore to editor" action, and rollback
- **Tool scopes**: `ToolScopesSection` with `ToolScopeForm` — per-agent
  allow/deny with glob support compiled to Cedar server-side
- **Hot-reload banner**: SSE subscription to `/api/events` surface
  `policy_reload_ok` and `policy_reload_error` events in real time

This is substantively more complete than the beta-readiness plan assumed.
The admin UI already has a functional policy editor.

### What is missing

Two smaller gaps:

1. **No syntax highlighting** — the policy textarea is a plain `<Textarea>`.
   Writing Cedar in a bare textarea without keyword highlighting or indentation
   hints is error-prone for complex policies. A CodeMirror or Monaco editor
   integration would be a meaningful UX improvement.

2. **Approval queue is not surfaced from the policy editor** — there is no
   link from a policy with `@require_approval` to the current pending approvals
   it has generated. The Approvals page exists (`Approvals.tsx`) but the two
   surfaces are not connected.

**Effort estimate:** 1–2 changes (syntax highlighting; approval→policy linkage)

---

## Goal 4 — Metrics and observability documentation

**Status: NOT MET — metrics exist but are undocumented**

### Current state

`crates/flint-gate-core/src/metrics.rs` is a clean, well-tested implementation
exposing 6 named metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `flint_delegate_total` | counter | Delegate-exchange outcomes (`result` label) |
| `flint_local_exchange_total` | counter | Local token-mint outcomes (`result` label) |
| `flint_delegate_latency_seconds` | histogram | Delegate-exchange latency |
| `flint_tool_authz_total` | counter | Per-tool-call authz decisions (`decision` label) |
| `flint_agent_budget_denied_total` | counter | Agent budget denials |
| `flint_governance_reload_rejected_total` | counter | Hot-reload rejections (governance lint) |

The metrics are served on the admin port at `GET /metrics` (via
`PrometheusHandle::render`). Tests in `metrics.rs` verify that each metric is
rendered with its labels.

### What is missing

- No `docs/docs/metrics.md` exists. The doc site has no metrics reference at all.
- No Grafana dashboard definition (`grafana/dashboard.json` or similar) exists.
- The `operations.md` added in the prior phase mentions `GET /api/policies/reload-status`
  but not `GET /metrics` or any of the metric names.

**Effort estimate:** 1 change (metrics reference page + optional Grafana JSON)

---

## Goal 5 — Agent SDK enhancements

**Status: NOT MET — both SDKs are functional but lack production hardening**

### Go SDK gaps (in `sdks/go/`)

The Go SDK (`client.go`, `stream.go`, `ws.go`) handles:
- All admin CRUD operations
- SSE streaming via `StreamSSE`
- WebSocket via `ws.go`
- `APIError` struct with `StatusCode` and `Body`

**Missing:**

1. **No automatic token refresh** — `Options` accepts a static `Token` string.
   When the JWT expires, the caller gets a 401 and must handle it themselves.
   No `TokenSource` interface or refresh hook exists.

2. **No retry-on-429** — `doJSON` returns on non-2xx without retry. A
   rate-limited response (429) is returned to the caller as an `APIError`.
   Production agent frameworks need automatic backoff on 429.

3. **No structured error types beyond `APIError`** — there is no
   `IsRateLimited(err)`, `IsUnauthorized(err)`, or `IsApprovalRequired(err)`
   helper beyond `IsNotFound`. Callers must inspect raw status codes.

4. **`StreamSSE` has no reconnect on network error** — if the SSE connection
   drops mid-stream, the channel closes. No automatic reconnect with
   exponential backoff is implemented.

### TypeScript SDK gaps (in `sdks/typescript/src/`)

The TypeScript SDK mirrors the Go SDK in structure. Similar gaps exist:

1. **No token refresh** — `FlintGateClientOptions` accepts a static `token`
   string. No refresh callback / `TokenProvider` pattern.

2. **No 429 retry** — `doJSON` in `client.ts` returns a structured error on
   non-2xx without retry.

3. **No `isRateLimited()`, `isUnauthorized()` helpers** — only generic error
   objects.

4. **No SSE reconnect** — `streamSSE` in `stream.ts` closes the iterator on
   network error without reconnect.

**Effort estimate:** 2–3 changes per SDK (token refresh + retry + error helpers;
SSE reconnect as a separate optional change)

---

## Summary Gap Table

| Goal | Status | Severity | Effort |
|------|--------|----------|--------|
| G-1: Postgres approval store | NOT MET | HIGH — correctness on pod restart | 3–4 changes |
| G-2: CI integration tests | PARTIALLY MET | LOW — TypeScript expiry test + fixture config | 1 change |
| G-3: Admin UI policy editor | SUBSTANTIALLY MET — gaps are UX, not functionality | LOW — syntax highlighting, approval linkage | 1–2 changes |
| G-4: Metrics documentation | NOT MET | MEDIUM — operator visibility | 1 change |
| G-5: SDK enhancements | NOT MET | MEDIUM — production agent framework friction | 4–6 changes (2–3 per SDK) |

---

## What Is Working Well

- **`Policies.tsx` is a complete policy management UI** — create, edit, delete,
  version history with diff view, rollback, tool-scope builder, live reload
  banner. Goal 3 is substantially closer to "done" than the goals.md implied.

- **Integration tests are wired to CI** — `integration.yml` runs on PR
  automatically against a real stack. This is a strong foundation; the gap is
  narrow.

- **Metrics implementation is correct and tested** — 6 metrics with
  stable low-cardinality label sets. Documentation is the only missing piece.

- **`ApprovalManager` has a clear trait surface** — the existing methods
  (`register`, `decide`, `list`, `status`, `purge_expired`, `earliest_expiry`)
  are exactly the interface a Postgres backend would need to implement. The
  abstraction work is low-risk.

---

## Recommended Change Order for Planning

1. **G-4: `add-metrics-docs`** — smallest change, highest operator value per
   line of work. One doc page closes a gap that every production deployment
   will hit.

2. **G-2: `fix-approval-expiry-ci`** — verify TypeScript approval expiry test,
   confirm test fixture uses short TTL. Small change, eliminates a CI blind spot.

3. **G-3: `add-policy-editor-syntax-highlight`** — CodeMirror/Monaco integration
   for the Cedar textarea; optionally link pending approvals to their originating
   policy.

4. **G-1: `add-postgres-approval-store`** — trait extraction + Postgres impl +
   migration + config wiring. This is the most impactful correctness change.
   Should be split into: (a) trait abstraction, (b) Postgres implementation,
   (c) config wiring + migration.

5. **G-5: SDK hardening** — token refresh + 429 retry + error helpers for Go,
   then TypeScript. SSE reconnect as a follow-on if scope allows.
