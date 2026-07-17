# Proposal — add-cedar-hot-reload-observability

**Phase:** cedar-policy-authoring-ux
**Goal:** G1 (HIGH)
**Build position:** 2 of 4 — depends on `PolicyParseError` type from change 1.

## Problem

The NOTIFY-triggered async policy reload (`cache/mod.rs:366–374` →
`reload_from_records_lenient`) silently swallows row-level parse errors as
`warn!()` logs. A DB failure on reload is logged at `error!()` but never
surfaces beyond the log. `AuthzEngine` carries no stored last-reload status.
No admin endpoint can query reload health. The admin server has no SSE channel.
Additionally, startup silently degrades to default-deny if the initial DB policy
load fails, with no operator-visible signal beyond logs.

## Solution

1. Add `last_reload_status: Arc<Mutex<ReloadStatus>>` to `AuthzEngine`; update
   it on every reload attempt (success or failure with structured error list).
2. Add `AdminEvent` enum with `PolicyReloadError { skipped: Vec<PolicyParseError>, db_error: Option<String> }` and `PolicyReloadOk { policy_count: usize }` variants.
3. Add `admin_events: tokio::sync::broadcast::Sender<AdminEvent>` to `AdminState`; thread it from `main.rs` to the cache/reload path.
4. NOTIFY reload path emits `AdminEvent` after every reload (success or failure).
5. Add `GET /events` SSE endpoint on admin router that streams `AdminEvent`s as newline-delimited JSON.
6. Add `GET /policies/reload-status` endpoint returning `{ ok: bool, last_error: Option<String>, policy_count: usize, last_reload_at: Option<String> }`.
7. Add `require_policies_at_startup: bool` to `ApprovalConfig` (or `ServerConfig`); when `true`, `main.rs` refuses to start if `from_database_with_sugar` returns an empty/failed engine.

## Scope

- `crates/flint-gate-core/src/authz/engine.rs` — `last_reload_status` field + `ReloadStatus` type
- `crates/flint-gate-core/src/admin/mod.rs` — `AdminEvent` enum; `admin_events` broadcast field on `AdminState`; `GET /events` SSE handler; `GET /policies/reload-status` handler
- `crates/flint-gate-core/src/cache/mod.rs` — emit `AdminEvent` after NOTIFY-triggered reload; pass `broadcast::Sender` through to reload path
- `crates/flint-gate-core/src/config/types.rs` — `require_policies_at_startup: bool` (default `false`)
- `crates/flint-gate/src/main.rs` — create `broadcast::channel`; thread sender through `AdminState` and cache; check `require_policies_at_startup` at startup
- `config.example.yaml` — document `require_policies_at_startup`

## Acceptance criteria

- When the NOTIFY-triggered reload encounters a bad policy row, an `AdminEvent::PolicyReloadError` is broadcast and a connected SSE client receives a JSON event.
- `GET /policies/reload-status` returns `{ ok: false, last_error: "...", ... }` after a failed reload.
- `GET /policies/reload-status` returns `{ ok: true, policy_count: N, ... }` after a successful reload.
- With `require_policies_at_startup: true` and an empty DB, `main.rs` exits non-zero before binding ports.
- With `require_policies_at_startup: false` (default), existing startup behavior is unchanged.
- `GET /events` SSE endpoint streams events to multiple concurrent subscribers (broadcast semantics).
- 4+ new tests: reload-error emits event, reload-ok emits event, reload-status endpoint, startup-fail-closed flag.
