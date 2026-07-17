# Assessment — cedar-policy-authoring-ux

_Assessed: 2026-07-08_
_Assessor: kbd-assess (codebase scan via Explore agent)_

---

## Starting State

The Cedar policy engine (`cedar-policy = "4"`) is fully wired with hot-reload
and write-time validation from prior phases. The engine is correct and
production-hardened. The admin API exposes `GET/POST/PUT/DELETE /policies`
routes. A React 19 + Vite + TypeScript admin UI exists with a raw `<Textarea>`
policy editor. The gaps are entirely in the **operator-facing surface layer**:
errors are not structured, not observable, and not surfaced to the UI in a
useful way.

---

## Gap Analysis by Goal

### G1 — Hot-reload error recovery (HIGH)

**Current state:**
- **Strict reload** (`reload_from_records`, `engine.rs:304–317`): parse-before-swap.
  A bad policy bundle returns `Err(AuthzError)`, last-good bundle is retained
  atomically, and `error!()` is logged. The admin write path (`admin/mod.rs:795–815`)
  checks this `Err` and returns HTTP 500 with `stored_but_not_activated`. This
  path is correct and observable.
- **Lenient NOTIFY reload** (`reload_from_records_lenient`, triggered by
  `cache/mod.rs:366–374`): individual bad rows are silently skipped with
  `warn!()`. A DB failure from `db.load_enabled_policies()` is logged at
  `error!()` but not broadcast anywhere. The engine silently retains the
  last-good bundle with no external observable state.

**Gaps:**
- NOTIFY-triggered lenient reload emits only `warn!()`/`error!()` log lines on
  errors — no structured event, no broadcast, no stored last-reload status.
- Cedar 4.x `ParseErrors` exposes `SourceLocation` with byte offsets (mappable
  to line/column) but every call site calls `.to_string()` immediately, losing
  the structure (`bundle.rs:181`, `validator.rs:32, 67`).
- `AuthzEngine` carries no `last_reload_error` field; no admin endpoint can
  query reload health without tailing logs.
- Startup DB policy load (`main.rs:308–317`) does not `?`-propagate failure:
  on DB error the engine starts with an empty default-deny bundle. The process
  starts successfully even with zero policies loaded. There is no
  `strict_policy_load` startup flag.

**Required work:**
- Add `last_reload_error: Arc<Mutex<Option<PolicyReloadStatus>>>` to
  `AuthzEngine` (or a dedicated `ReloadStatusStore`).
- Emit a structured event on every NOTIFY-reload outcome (success or failure)
  to an `AdminEvent` broadcast channel in `AdminState`.
- Add `GET /admin/events` SSE endpoint that streams `AdminEvent`s.
- Optionally: expose `GET /policies/reload-status` for polling clients.
- Startup: add a `fail_on_empty_policies: bool` config flag; when set, refuse
  to start if `from_database_with_sugar` produces an empty/failed engine.

---

### G2 — `POST /policies/validate` endpoint (HIGH)

**Current state:**
- No dry-run validate endpoint exists. Route table (`admin/mod.rs:118–184`)
  has no `/policies/validate` or `/policies/simulate` entry.
- Write-time validation is embedded inside `upsert_policy_inner`
  (`admin/mod.rs:731–815`): the policy text is parsed and validated as a side
  effect of saving. The validation logic is not extracted to a reusable
  function.
- The error returned on invalid policy is an HTTP 422/400 with body
  `{ "error": "invalid_policy", "message": "<flat string>" }` — the flat
  string is the output of `e.to_string()` on the Cedar `ParseErrors` type.

**Gaps:**
- No pure validation endpoint (dry-run, no side effects).
- No structured error response: Cedar 4.x `ParseErrors` implements
  `Iterator<Item = ParseError>` where each `ParseError` has a
  `source_location() -> Option<Loc>` returning start/end byte offsets. None of
  this is extracted.
- The existing `AuthzError::PolicyParse(String)` and
  `AuthzError::ValidationErrors(Vec<String>)` variants carry flat strings only
  (`error.rs:10–41`).

**Required work:**
- Extract validation logic from `upsert_policy_inner` into
  `validate_policy_text(text: &str, schema: Option<&Schema>) -> Result<(), Vec<PolicyParseError>>`.
- Define `PolicyParseError { line: usize, column: usize, message: String }`.
- Wire Cedar 4.x `ParseErrors::iter()` + `source_location()` to populate
  `PolicyParseError`.
- Add `POST /policies/validate` handler: accepts `{ policy: String, schema: Option<String> }`,
  returns `{ valid: bool, errors: Vec<PolicyParseError> }`.
- Rate-limited by the existing `AdminGovernorLayer` (no special casing needed).

---

### G3 — Admin UI inline Cedar parse error surface (MEDIUM)

**Current state:**
- `web/src/pages/Policies.tsx` has a `<Textarea rows={8}>` for raw Cedar
  policy text (line 356).
- On submit error, `toast({ title: 'Failed to save policy', description: ... })`
  fires (`Policies.tsx:334–343`). The error is a transient toast — gone after a
  few seconds, no inline marker.
- No live validation, no syntax highlighting, no line/column annotation, no
  "validate" button separate from "save".

**Gaps:**
- No call to a `/validate` endpoint before or during editing.
- No inline error display tied to the editor.
- No CodeMirror/Monaco integration (not required for MVP — even a simple error
  list below the textarea with line numbers is sufficient given the goals).

**Required work:**
- Add `POST /policies/validate` call (debounced, ~500ms) triggered on
  `<Textarea>` change.
- Render `errors[]` from the response as an inline list below the editor
  (`line N, col M: <message>`).
- Disable the Save button when `valid === false`.
- Add a standalone "Validate" button for explicit on-demand validation.
- (Optional stretch): wire CodeMirror 6 with a Cedar language mode for
  syntax highlighting — assess whether a Cedar language definition exists
  before committing to this.

**Framework note:** React 19 + TanStack Query v5 — `useMutation` for the
validate call, debounce via `useEffect` + `setTimeout`.

---

### G4 — `POST /policies/simulate` endpoint (MEDIUM)

**Current state:** Does not exist. `AuthzEngine::is_authorized` exists
(`engine.rs`) and evaluates a Cedar request against the current live policy
bundle. However:
- It is not exposed via any admin endpoint.
- There is no way for an operator to send a test Cedar request and see
  Allow/Deny + which rule matched without sending live traffic.

**Cedar 4.x `is_authorized` return:** `AuthorizationEngine::is_authorized()`
returns `Response` with `decision()` (Allow/Deny) and `diagnostics()`
(reasons + errors). The `reason()` method on `Response` gives the set of
policy IDs that contributed. This is already structured — it just needs to be
exposed.

**Gaps:**
- No `/simulate` route.
- The Cedar request context shape (principal, action, resource, context map)
  is not currently serializable from JSON in a single place — it is assembled
  from gateway-specific types in the authz module. A new
  `SimulateRequest { principal, action, resource, context }` DTO is needed.

**Required work:**
- Define `SimulateRequest` and `SimulateResponse { decision, reasons, errors }` DTOs.
- Add `POST /policies/simulate` handler that calls `authz_engine.is_authorized()` directly.
- No persistence, no side effects, no live traffic path.
- Option: support `policy_override: Option<String>` in the request body to
  simulate against a caller-supplied policy text instead of the live bundle —
  this is the more powerful form but adds Cedar bundle construction in the handler.
  Recommend: ship live-policy-only first (simpler, no security risk), defer
  override to a follow-up.

---

## Open Questions — Resolved by Assess

| Question | Resolution |
|----------|------------|
| Cedar SDK error format | **Structured.** Cedar 4.x `ParseErrors` has `Iterator<Item=ParseError>` with `source_location() -> Option<Loc>` (byte offsets). Line/column derivable from the source string. Worth extracting. |
| SSE vs. polling for hot-reload errors | **SSE preferred** — the admin router has no SSE infrastructure yet, so the admin events channel is new work. Both options are viable; SSE is better for a real-time UI. Polling via `GET /policies/reload-status` is the simpler fallback. Recommend: add both (SSE channel + status endpoint). |
| `/simulate` scope (live policy vs. caller-supplied) | **Live policy only for this phase.** Caller-supplied policy text simulation adds bundle construction + security review overhead; defer to next iteration. |
| Admin UI framework | **React 19 + Vite + TypeScript + TanStack Query v5.** SPA embedded as `RustEmbed` static assets. Inline error surface is straightforward with debounced `useMutation`. CodeMirror 6 is optional stretch. |

---

## Items Confirmed Not Required This Phase

- `strict_policy_load` startup flag: **include** — the assessment found that
  startup silently degrades to default-deny on DB failure. A config flag
  (`require_policies_at_startup: bool`) is a small addition to G1 scope and
  prevents a class of invisible misconfiguration.
- Cedar entity/schema simulation: the `/simulate` endpoint will focus on policy
  evaluation with minimal dummy entities. Full entity graph simulation is out of
  scope.
- Policy versioning / rollback history: out of scope (confirmed).
- CodeMirror/Monaco integration: optional stretch in G3; not a success criterion.

---

## Revised Success Criteria

- [ ] **G1a**: NOTIFY-triggered reload emits `AdminEvent::PolicyReloadError` on
      skipped rows or DB failure; event is broadcast to SSE channel.
- [ ] **G1b**: `GET /admin/events` SSE endpoint streams `AdminEvent`s to the
      admin UI.
- [ ] **G1c**: `GET /policies/reload-status` returns `{ ok: bool, last_error: Option<String>, last_reload_at: Option<DateTime> }`.
- [ ] **G1d**: `require_policies_at_startup` config flag; when `true`, gateway
      refuses to start if initial policy load returns an empty/failed engine.
- [ ] **G2**: `POST /policies/validate` returns `{ valid, errors: [{ line, column, message }] }`;
      Cedar 4.x `SourceLocation` extracted (not `.to_string()`).
- [ ] **G3**: Admin UI policy editor calls `/validate` on change (debounced),
      shows inline errors, disables Save on invalid.
- [ ] **G4**: `POST /policies/simulate` accepts `{ principal, action, resource, context }`
      Cedar request and returns `{ decision: "Allow"|"Deny", reasons: [...], errors: [...] }`.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; ≥80%
      coverage on new code; web build green.

---

## Recommended Build Order

1. **G2** (`POST /policies/validate`) — foundation. Extracts Cedar error
   structure; all other goals benefit from this work.
2. **G1** (hot-reload error recovery + SSE + reload-status) — depends on
   `AdminEvent` type, which benefits from the `PolicyParseError` type introduced
   in G2.
3. **G3** (admin UI inline errors) — depends on G2 endpoint being live.
4. **G4** (`POST /policies/simulate`) — independent of G1–G3; can be built in
   parallel with G3 but logically follows G2 since both are new admin endpoints.
