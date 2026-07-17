# add-written-by-to-policy-list

## Summary

Surface `written_by` (last-modified-by) in the policy list table and wire JWT `sub` attribution into the policy upsert path.

## Why

`cedar_policy_versions.written_by` is populated in DB but was left as `None`/null in the upsert handler (comment in `db/mod.rs:1109` deferred attribution). The `authz_policies` table has no `written_by` column; the most recent author must be JOIN'd from `cedar_policy_versions`. Without attribution, the policy list gives operators no signal about who last changed each policy.

## What Changes

### Backend (Rust)

- `crates/flint-gate-core/src/db/mod.rs`:
  - Modify `list_policies()` query to LEFT JOIN `cedar_policy_versions` on the latest version per policy (using a lateral subquery or `DISTINCT ON`); add `written_by: Option<String>` to `PolicyRow` struct and `from_row`.
  - Modify `upsert_policy(id, text, schema, entities, enabled, written_by)` signature to accept `written_by: Option<&str>`; propagate to the `cedar_policy_versions` insert.
- `crates/flint-gate-core/src/admin/mod.rs`:
  - Extract JWT `sub` claim from the request in `upsert_policy_inner` (admin auth middleware already validates the JWT; `sub` is available in the `AdminClaims` extension).
  - Pass `Some(sub)` as `written_by` to `db.upsert_policy(...)`.

### Frontend (TypeScript)

- `web/src/api/types.ts` — add `written_by?: string | null` to `PolicyRow` interface.
- `web/src/pages/Policies.tsx`:
  - Add `<TableHead>Last by</TableHead>` to the policy table header.
  - Add `<TableCell>{policy.written_by ?? '—'}</TableCell>` to `PolicyTableRow`.
