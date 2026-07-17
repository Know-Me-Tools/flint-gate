# Assessment — cedar-policy-versioning-and-rollback

_Generated: 2026-07-08_

## Baseline Health

| Check | Result |
|-------|--------|
| `cargo test --workspace` | **PASS** — 515 + 22 subsidiary tests, 0 failures |
| `cargo clippy -D warnings` | Assumed PASS (prior phase ended clean) |
| Web build | Assumed PASS (prior phase ended clean) |

---

## Goal-by-Goal Gap Analysis

### G1 — Policy version history schema (HIGH)

**Status: NOT STARTED — 0% complete**

**What exists:**
- `authz_policies` table in `SCHEMA_SQL` (`db/mod.rs:63-71`): `id TEXT PK, policy_text TEXT, schema_json JSONB, entities_json JSONB, enabled BOOL, created_at, updated_at`. Upsert via `Database::upsert_policy` (`db/mod.rs:1077-1103`) — plain `INSERT … ON CONFLICT DO UPDATE`, no version row written.
- Schema is applied as a single inline `SCHEMA_SQL` constant via `sqlx::raw_sql` at startup (`db/mod.rs:390`). No migration framework; DDL is idempotent `CREATE TABLE IF NOT EXISTS`.

**What is missing:**
- `cedar_policy_versions` table does not exist anywhere in `SCHEMA_SQL` or any `.sql` file.
- `Database::upsert_policy` does not write a history row; the prior `policy_text` is overwritten and lost.
- No `version_num` sequence per `policy_id`.
- No `written_by` attribution column.
- The schema extension mechanism is the existing `SCHEMA_SQL` constant — adding `CREATE TABLE IF NOT EXISTS cedar_policy_versions (…)` there is the correct approach (idempotent, no migration runner needed).

**Gap:** Entire schema missing. Must add before any other goal can proceed.

---

### G2 — Admin API: `GET /policies/{id}/history` (HIGH)

**Status: NOT STARTED — 0% complete**

**What exists:**
- Route table in `admin/mod.rs:169-207` registers: `/policies`, `/policies/validate`, `/policies/{id}` (GET/PUT/DELETE), `/policies/reload-status`, `/policies/simulate`. No `/policies/{id}/history` route.
- `Database::get_policy`, `list_policies`, `upsert_policy`, `delete_policy` exist; no `list_policy_versions` or `get_policy_history` method exists.
- `PolicyRow` struct (`db/mod.rs:1202-1211`) has no `version_num` or `written_at` field.

**What is missing:**
- `Database::list_policy_versions(id, offset, limit)` method.
- `PolicyVersionRow` struct.
- `list_policy_history_handler` async fn.
- Route `.route("/policies/{id}/history", get(list_policy_history_handler))` registered BEFORE `/{id}` wildcard.

**Critical ordering note:** `/policies/{id}/history` must be registered BEFORE `/policies/{id}` in the Axum router — same lesson as `validate` and `simulate` in the prior phase.

**Gap:** Route, handler, and DB method all missing.

---

### G3 — Admin API: `POST /policies/{id}/rollback` (HIGH)

**Status: NOT STARTED — 0% complete**

**What exists:**
- `upsert_policy_inner` (`admin/mod.rs:985-1065`) is the authoritative write path: validates with Cedar before writing (`validate_policy(&record)` at line 1023), then calls `db.upsert_policy(…)` and `authz.reload_from_database(db)`. This is the pattern rollback must reuse — not bypass.
- Cedar validation is done via `validate_policy(&PolicyRecord)` (pre-write). Rollback must also call this before writing the restored version (fail-closed: an invalid version in history must not be reloadable).

**What is missing:**
- `RollbackRequest { version_num: i32 }` struct.
- `RollbackResponse { status, policy_id, from_version, to_version }` struct.
- `rollback_policy_handler` async fn — must: (a) fetch target version row from `cedar_policy_versions`, (b) Cedar-validate the restored text (reject 422 if invalid), (c) call `db.upsert_policy(…)` (which also writes a new version row — the rollback is itself a version), (d) call `authz.reload_from_database(db)`, (e) return structured response.
- Route `.route("/policies/{id}/rollback", post(rollback_policy_handler))` registered before `/{id}`.
- `Database::get_policy_version(policy_id, version_num)` method.

**Gap:** Full implementation missing. Pattern for handler is established by `upsert_policy_inner`; rollback is a constrained variant of it.

---

### G4 — Admin UI: version history panel (MEDIUM)

**Status: NOT STARTED — 0% complete**

**What exists (`web/src/pages/Policies.tsx`):**
- `PolicyForm` component: full editor with debounced validation, inline errors, valid indicator, save/delete buttons.
- `validatePolicy` imported from `@/api/admin`.
- No history panel, no rollback button, no `fetchPolicyHistory` or `rollbackPolicy` calls anywhere.

**What exists (`web/src/api/admin.ts` / `types.ts`):**
- `PolicyParseError`, `ValidateResponse`, `AdminEvent`, `ReloadStatus` types.
- No `PolicyVersionRow`, `PolicyHistoryResponse`, `RollbackResponse` types.
- No `fetchPolicyHistory()` or `rollbackPolicy()` functions.

**What is missing:**
- `PolicyVersionRow` and `PolicyHistoryResponse` types in `types.ts`.
- `fetchPolicyHistory(id, offset?, limit?)` and `rollbackPolicy(id, versionNum)` in `admin.ts`.
- In `Policies.tsx`: a "Version History" collapsible section in `PolicyForm` (or a separate panel) showing version list, "View" (read-only textarea), "Rollback" button with confirmation dialog.
- After rollback: refresh the policy editor to show the restored text + trigger a success toast.

**Gap:** All UI, API functions, and types missing.

---

## Dependency Graph

```
G1 (schema) ─────────────────────────────────────────────────────┐
    │                                                              │
    ├─→ G2 (history endpoint) ─→ G4 (UI history panel)          │
    │                              (reads G2's endpoint)          │
    └─→ G3 (rollback endpoint) ─→ G4 (UI rollback button)        │
         (also writes a new version row via G1 schema)            │
```

Build order: G1 → G2 → G3 → G4.

---

## Technical Analysis

### Schema extension approach (G1)

The project uses a single `SCHEMA_SQL` constant with `CREATE TABLE IF NOT EXISTS` DDL applied idempotently at startup. There is no migration runner (no `sqlx-migrate`, no Flyway). Adding the new table follows the same pattern:

```sql
CREATE TABLE IF NOT EXISTS cedar_policy_versions (
    id           SERIAL PRIMARY KEY,
    policy_id    TEXT NOT NULL REFERENCES authz_policies(id) ON DELETE CASCADE,
    version_num  INT NOT NULL,
    policy_text  TEXT NOT NULL,
    schema_json  JSONB,
    entities_json JSONB,
    written_by   TEXT,
    written_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (policy_id, version_num)
);

CREATE INDEX IF NOT EXISTS cedar_policy_versions_policy_id_idx
    ON cedar_policy_versions (policy_id, version_num DESC);
```

**Version numbering:** Application-level (`SELECT COALESCE(MAX(version_num), 0) + 1 FROM cedar_policy_versions WHERE policy_id = $1`) inside a transaction with `upsert_policy`. Postgres sequences per-policy_id would be cleaner but require DDL per row; the `MAX+1` approach is sufficient and simpler given low write volume.

**`written_by` sourcing:** The current `upsert_policy_inner` in `admin/mod.rs` does not thread caller identity through. For this phase, `written_by` will be captured from the `AdminState` or left `NULL` — investigate whether the admin JWT or API key identity is accessible in the Axum handler state before committing. If not easily available, accept `NULL` and treat it as a deferred open question.

**`ON DELETE CASCADE`:** Deleting a policy from `authz_policies` cascades to all its version rows. This is correct — there is no point keeping orphaned version history after a policy is fully deleted.

### Versioning in `upsert_policy` (G1/G3)

The `Database::upsert_policy` method will be extended to also insert a `cedar_policy_versions` row. The safest approach is to wrap both the main upsert and the version insert in a single transaction (as `insert_nhi_audit` already does for NHI events). This ensures the version row is never missing for a successfully upserted policy.

The `upsert_policy` signature should gain an optional `written_by: Option<&str>` parameter — callers that don't know the identity pass `None`.

### Route ordering (G2/G3)

From the prior phase (lesson learned): `/policies/{id}/history` and `/policies/{id}/rollback` MUST be registered before `/policies/{id}`. Looking at `admin/mod.rs:173-207`, the current ordering is:

```
.route("/policies/validate", post(…))    ← literal; fine, no {id}
.route("/policies/{id}", get/put/delete)  ← wildcard
.route("/policies/reload-status", get(…)) ← BUG RISK — registered AFTER {id}!
.route("/policies/simulate", post(…))     ← BUG RISK — registered AFTER {id}!
```

Wait — looking more carefully: `reload-status` and `simulate` are registered at lines 206-207, AFTER `/{id}` at line 175. These work in the current codebase because Axum's router uses a priority system (literal segments beat parameterized ones regardless of registration order). Confirm this holds for `/policies/{id}/history` as well: since `history` is a literal path segment in the second position (`/{id}/history`), Axum will correctly match it before `/{id}` for requests to `.../some-id/history`. Same for `rollback`.

Still, follow the registration order established for `validate` and `simulate` — register the literal sub-routes before the base wildcard for clarity.

### Fail-closed rollback (G3)

The rollback handler MUST call `validate_policy(&record)` before writing the restored text. A prior version row could be semantically invalid if (a) Cedar's validation rules changed between when it was written and when it is restored, or (b) the row was manually tampered with. Rejecting invalid rollback targets with 422 keeps the fail-closed invariant.

### Admin UI framework (G4)

Existing stack: React 19 + Vite + TypeScript + TanStack Query v5. The confirmation dialog for rollback can use the existing shadcn/radix Dialog pattern already used in the approvals page, or a simple `window.confirm()` — prefer the former to match the existing design system. The version list is a simple read-only table; no additional libraries needed.

---

## Open Questions (Resolved for Plan)

| Question | Resolution |
|----------|-----------|
| Trigger vs. application-level versioning? | **Application-level** — insert in a transaction alongside `upsert_policy`. Easier to test (no Postgres trigger in `SCHEMA_SQL`), consistent with the NHI audit pattern already in the codebase. |
| `written_by` attribution? | **`NULL` for now** — the admin handlers don't currently thread JWT subject or API key client_id into the DB layer. Add the column as nullable; a future phase can wire identity. Document as tech debt. |
| Version retention? | **Keep all** — low write volume (human operators, not automated loops). No cap needed for this phase. |
| Route ordering safety for `/{id}/history`? | **Safe** — Axum literal-segment priority beats parameterized regardless of registration order, but register before `/{id}` for consistency. |
| Cedar re-validation on rollback? | **Yes, mandatory** — call `validate_policy` before writing, return 422 if invalid. Never write an invalid policy to `authz_policies`. |

---

## Planned Changes (input to /kbd-plan)

| # | Change ID | Priority | Description |
|---|-----------|----------|-------------|
| 1 | `add-policy-version-history-schema` | HIGH | Add `cedar_policy_versions` table to `SCHEMA_SQL`; extend `Database::upsert_policy` to write a version row in a transaction; add `PolicyVersionRow` struct and `Database::list_policy_versions` / `Database::get_policy_version` methods; tests for version insertion. |
| 2 | `add-policy-history-endpoint` | HIGH | Add `GET /policies/{id}/history` handler + DB method; paginates with `?offset=N&limit=N`; returns `{ policy_id, versions: [...] }`; 4+ tests. |
| 3 | `add-policy-rollback-endpoint` | HIGH | Add `POST /policies/{id}/rollback` handler; fetches version, Cedar-validates, upserts, reloads, returns structured response; returns 404/422 on bad input; 4+ tests. |
| 4 | `add-policy-version-history-ui` | MEDIUM | Add `PolicyVersionRow` + `PolicyHistoryResponse` types; add `fetchPolicyHistory` + `rollbackPolicy` to `admin.ts`; add collapsible history panel with view + rollback button + confirmation to `Policies.tsx`. |

**Total: 4 changes** (same structure as the prior phase's 4 changes).

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `upsert_policy` transaction wrapping breaks existing tests | LOW | HIGH | Wrap inside transaction while keeping same signature; mock tests don't hit DB. |
| Axum route ordering issue for `/{id}/history` | LOW | HIGH | Register before `/{id}` and add a route-disambiguation test. |
| Rollback of a valid-at-write but invalid-now policy | MEDIUM | CRITICAL | Cedar-validate before writing — already mitigated by design. |
| `written_by` NULL leaves no attribution trail | LOW | LOW | Documented as tech debt; deferred. |
| `MAX(version_num)+1` race condition | LOW | LOW | Wrapped in a transaction with the upsert; serialized at the DB level. |
