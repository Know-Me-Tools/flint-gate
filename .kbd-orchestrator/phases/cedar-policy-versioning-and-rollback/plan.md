# Plan — cedar-policy-versioning-and-rollback

_Backend: OpenSpec | Driver: kbd-apply_

## Ordered Change List

Changes are ordered by dependency: each change's runtime behavior depends on
the previous one existing. All 4 changes must be executed in sequence.

| Order | Change ID | Priority | Goals | Estimated Tasks |
|-------|-----------|----------|-------|-----------------|
| 1 | `add-policy-version-history-schema` | HIGH | G1 | 7 |
| 2 | `add-policy-history-endpoint` | HIGH | G2 | 5 |
| 3 | `add-policy-rollback-endpoint` | HIGH | G3 | 5 |
| 4 | `add-policy-version-history-ui` | MEDIUM | G4 | 6 |

## Change Summaries

### Change 1 — `add-policy-version-history-schema`

**Purpose:** Data layer prerequisite — everything else builds on this.

Add `cedar_policy_versions` table to `SCHEMA_SQL` (idempotent). Extend
`Database::upsert_policy` to wrap both the main policy upsert and a version
insert in a single Postgres transaction. Add `PolicyVersionRow` struct,
`Database::list_policy_versions`, and `Database::get_policy_version` methods.
Update all 4 call sites of `upsert_policy` in `admin/mod.rs` to pass the new
`written_by: None` parameter.

**Key design decisions:**
- Application-level `MAX(version_num)+1` inside a transaction (not a trigger) — consistent with `insert_nhi_audit` pattern.
- `ON DELETE CASCADE` on the FK so hard-deleting a policy removes its history.
- `written_by` is nullable; attribution wiring is deferred tech debt.

**Tests:** First write → version_num=1; second write → version_num=2;
`get_policy_version` returns correct row; cascade delete verified.

---

### Change 2 — `add-policy-history-endpoint`

**Purpose:** Surface version history over HTTP.

Add `GET /policies/{id}/history?offset=0&limit=20` to the admin API.
Registered before the `/{id}` wildcard. Returns 404 when the policy doesn't
exist; 200 with `PolicyHistoryResponse` (version list, newest first).

**Key design decisions:**
- Limit clamped server-side to 100 (not trusting caller).
- `total_hint: null` — no `COUNT(*)` for now; deferred.
- Route registration order: literal `/{id}/history` before `/{id}` wildcard.

**Tests:** 404 on unknown policy; empty list on fresh policy; DESC ordering;
pagination respected; limit > 100 clamped.

---

### Change 3 — `add-policy-rollback-endpoint`

**Purpose:** Make history actionable — restore a prior version.

Add `POST /policies/{id}/rollback` with body `{ version_num: N }`. Fetches
the target version, Cedar-validates the restored text (422 if invalid —
fail-closed), upserts it (creating a new version row for the rollback itself),
triggers hot-reload, returns `{ status, policy_id, from_version, to_version,
reloaded }`.

**Key design decisions:**
- Cedar validation is mandatory before write — a historically-valid policy
  that fails current validation is rejected with 422.
- The rollback is itself a new version (auditable undo).
- Reuses `validate_policy(&PolicyRecord)` already in scope in `admin/mod.rs`.

**Tests:** 404 on unknown policy; 404 on unknown version_num; 422 on
invalid Cedar text; happy path — new version_num = max+1, restored text
in DB, reloaded=true.

---

### Change 4 — `add-policy-version-history-ui`

**Purpose:** Make history and rollback accessible in the admin UI.

Add TypeScript types (`PolicyVersionRow`, `PolicyHistoryResponse`,
`RollbackResponse`) and API functions (`fetchPolicyHistory`,
`rollbackPolicy`). Add a collapsible "Version History" section to
`PolicyForm` in `Policies.tsx` with lazy loading, a version list, a
read-only "View" pane with "Restore to editor", and a
confirmation-gated "Rollback" button.

**Key design decisions:**
- History is lazy-loaded on expand (not on modal open) — avoids unnecessary
  network calls for operators who never open the panel.
- "Rollback" requires explicit confirmation — never single-click.
- "View → Restore to editor" copies text without saving — operator still
  must click Save, maintaining validation-before-write.

**Checks:** `tsc --noEmit` + `vite build` green.

---

## Execution Notes

### Do not skip validation on rollback

The rollback handler must call `validate_policy` before writing. This is a
hard constraint from the fail-closed security model — even a version that was
valid when written must pass current Cedar validation before it can be
restored. Return 422 with Cedar error details, never silently overwrite.

### Route registration order

For changes 2 and 3, the new routes (`/{id}/history`, `/{id}/rollback`) must
be registered before `/{id}` in the Axum router. Axum's literal-segment
priority makes this safe regardless of order, but explicit ordering matches
the established convention (`/validate` and `/simulate` before `/{id}`).

### Transaction discipline

Change 1 introduces a transaction in `upsert_policy`. The transaction must
commit atomically: both the policy row update AND the version row insert, or
neither. Any partial write (policy updated but no version row) would leave
the history table out of sync.

### Workspace health gate

Each change must end with `cargo test --workspace && cargo clippy --workspace -- -D warnings` green before archiving. Change 4 must also pass `tsc --noEmit` and `vite build`.

---

## Next Step

Run `/kbd-apply add-policy-version-history-schema` to start Change 1.
