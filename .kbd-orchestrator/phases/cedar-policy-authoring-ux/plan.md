# Plan — cedar-policy-authoring-ux

_Planned: 2026-07-08_
_Backend: OpenSpec_
_Changes: 4_

## Ordering Rationale

The assessment found that Cedar 4.x's `ParseErrors` iterator exposes `SourceLocation`
(byte offsets → line/column) but every call site in the codebase calls `.to_string()`
immediately, losing this structure. Extracting structured error types is the **foundation**
that unblocks all other goals:

- G2 (validate endpoint) introduces `PolicyParseError` + structured Cedar error extraction.
- G1 (hot-reload observability) uses `PolicyParseError` in `AdminEvent` and SSE.
- G3 (UI inline errors) consumes the `/validate` endpoint and SSE channel from G1/G2.
- G4 (simulate endpoint) is independent but shares the admin route infrastructure.

Build order: **G2 → G1 → G3 → G4**

---

## Change 1 of 4 — `add-cedar-policy-validate-endpoint` (G2, HIGH)

**Goal:** Extract structured Cedar error types and expose a dry-run validate endpoint.

**Key tasks:**
1. Define `PolicyParseError { line, column, length, message }` with a Cedar `SourceLocation` constructor.
2. Replace all `.to_string()` call sites in `bundle.rs` / `validator.rs` with structured extraction.
3. Extract `validate_policy_text()` from `upsert_policy_inner`.
4. Add `POST /policies/validate` → `{ valid: bool, errors: Vec<PolicyParseError> }`.
5. Tests (4+): valid policy, parse error with line/col, schema error, existing upsert still 422.

**Files:** `authz/error.rs`, `authz/bundle.rs`, `authz/validator.rs`, `admin/mod.rs`

---

## Change 2 of 4 — `add-cedar-hot-reload-observability` (G1, HIGH)

**Goal:** Make async NOTIFY-triggered reload failures observable via SSE and a status endpoint.

**Key tasks:**
1. Add `ReloadStatus` + `AdminEvent` types; `last_reload_status` on `AuthzEngine`.
2. Add `admin_events: broadcast::Sender<AdminEvent>` to `AdminState`; thread from `main.rs`.
3. Emit `AdminEvent` after every NOTIFY-triggered reload.
4. Add `GET /events` SSE endpoint + `GET /policies/reload-status` endpoint.
5. Add `require_policies_at_startup: bool` config flag (default `false`).
6. Tests (4+): reload error emits event, reload ok emits event, status endpoint, startup flag.

**Files:** `authz/engine.rs`, `admin/mod.rs`, `cache/mod.rs`, `config/types.rs`, `main.rs`, `config.example.yaml`

---

## Change 3 of 4 — `add-policy-editor-inline-errors` (G3, MEDIUM)

**Goal:** Admin UI policy editor shows inline Cedar parse errors and disables Save on invalid input.

**Key tasks:**
1. Add `validatePolicy` API call + TypeScript types.
2. Debounced validate-on-change (500ms); inline `Line N, Col M: <message>` error list.
3. Save button disabled when `valid === false`; "Policy is valid" indicator when valid.
4. Standalone "Validate" button for on-demand validation.
5. SSE hot-reload error banner via `EventSource` to `GET /events`.

**Files:** `web/src/api/admin.ts`, `web/src/pages/Policies.tsx`, `web/src/types/`

---

## Change 4 of 4 — `add-cedar-policy-simulate-endpoint` (G4, MEDIUM)

**Goal:** Expose Cedar authorization simulation for operator policy testing without live traffic.

**Key tasks:**
1. Define `SimulateRequest` / `SimulateResponse` DTOs.
2. Add `POST /policies/simulate` handler: parse EntityUid fields, call `is_authorized`, return structured decision.
3. Rate-limited by existing `AdminGovernorLayer`.
4. Tests (4+): allow, deny, invalid entity UID, reasons list.

**Files:** `admin/mod.rs` only

---

## Success Criteria (Phase)

- [ ] `POST /policies/validate` returns structured `{ valid, errors: [{ line, column, message }] }` with Cedar 4.x `SourceLocation` extracted.
- [ ] NOTIFY-triggered reload failures broadcast `AdminEvent` to SSE subscribers; `GET /events` streams them.
- [ ] `GET /policies/reload-status` reflects current reload health.
- [ ] `require_policies_at_startup: true` causes non-zero exit if initial policy load is empty/failed.
- [ ] Admin UI policy editor shows inline parse errors (line/col), disables Save on invalid, shows hot-reload error banner.
- [ ] `POST /policies/simulate` returns Allow/Deny + matched policy IDs without live traffic.
- [ ] `cargo check/clippy -D warnings/test --workspace` green; ≥80% coverage on new code; `web` build green.
