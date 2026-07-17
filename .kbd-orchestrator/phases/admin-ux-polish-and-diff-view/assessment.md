# Assessment — admin-ux-polish-and-diff-view

_Generated: 2026-07-09_
_Codebase state: post `cedar-policy-versioning-and-rollback` (all 4 changes archived)_

---

## Phase Goals Recap

1. Policy version diff view
2. Approvals auto-polling (≤5s)
3. History panel pagination (>20 versions)
4. `written_by` column in the policy list table

---

## Gap Analysis — Goal by Goal

### G1: Policy version diff view

**Status: NOT PRESENT — full gap**

Current state:
- `PolicyVersionHistory` in `web/src/pages/Policies.tsx:382` renders a table of versions. Each row has a "View" button that populates a read-only `<Textarea>` with the selected version's `policy_text` (`web/src/pages/Policies.tsx:519–540`).
- No diff is computed. The operator sees one version's text at a time in the read-only pane — there is no side-by-side or inline diff between any two versions.
- No diff library is installed (`package.json` has no `diff`, `jsdiff`, `monaco-editor`, or equivalent).

What needs to be built:
- A lightweight text-diff utility (either a small pure-JS `diff` library or a manual LCS implementation — avoid Monaco Editor; it's 2 MB+ and would blow the JS budget).
- UI: a "Compare" mode in the history panel where the operator selects two version rows and sees an inline unified or side-by-side diff. The simplest acceptable form is a unified diff rendered as colored `<pre>` lines (additions green, deletions red) — no editor widget needed.
- The diff should default to "selected version vs. current (latest)" when only one version is highlighted.

**Recommended library**: `diff` (npm package `diff@^7.0.0`, ~15 KB gzipped) — pure JS, no DOM dependency, produces standard unified / structured patch objects. Already used widely in the ecosystem.

---

### G2: Approvals auto-polling

**Status: PARTIALLY PRESENT — polling infrastructure removed, manual workaround exists**

Current state:
- `useApprovals` in `web/src/hooks/useAdmin.ts:248` exposes a `refetch` function (from `raw.refetch` on the entity list result).
- `web/src/pages/Approvals.tsx:86` calls `useApprovals()` but destructures only `{ data, isLoading, error }` — `refetch` is unused. There is no `useEffect` timer, no SSE subscription, no auto-refresh.
- The comment in `useAdmin.ts:257` notes: `// refetchInterval not in ListQueryOptions — handled by periodic polling below` — but no polling code follows. This is a stub comment left from the migration.

What needs to be built:
- A `useInterval` hook (or inline `useEffect` with `setInterval`) in `Approvals.tsx` that calls `refetch()` every 5 000 ms.
- The `refetch` return from `useApprovals` is already wired in the hook layer — just needs to be consumed by the page.
- No backend changes needed.

**Implementation complexity**: Small — ~10 lines in `Approvals.tsx`. No new dependencies.

---

### G3: History panel pagination

**Status: PARTIALLY PRESENT — API supports it, UI does not**

Current state:
- `fetchPolicyHistory(id, offset, limit)` in `web/src/api/admin.ts:187` accepts `offset` and `limit` parameters (default `offset=0, limit=20`) and passes them as query params.
- `GET /policies/{id}/history` backend returns `{ policy_id, total_hint, versions }` where `total_hint` is `Option<i64>` (nullable).
- `PolicyVersionHistory` in `Policies.tsx:396` calls `fetchPolicyHistory(policyId)` with no `offset`/`limit` — always fetches the first 20. The component state has `history: PolicyHistoryResponse | null` but no `offset`, `hasMore`, or append-on-load-more logic.
- `history.versions.length === 20` would be the only signal that more versions exist, but even that heuristic is not used.

What needs to be built:
- Track `offset` and `hasMore` in `PolicyVersionHistory` component state.
- On initial load, fetch page 0 (`offset=0, limit=20`).
- If `total_hint > 20` or `versions.length === 20`, show a "Load more" button at the bottom of the version table.
- Clicking "Load more" calls `fetchPolicyHistory(id, offset + 20, 20)` and appends results to `history.versions`.

**Implementation complexity**: Moderate — ~40 lines of state + JSX changes in `PolicyVersionHistory`.

---

### G4: `written_by` in policy list table

**Status: BACKEND GAP — `authz_policies` does not carry `written_by`; requires JOIN or schema addition**

Current state:
- `authz_policies` schema (`db/mod.rs:70`): `id, policy_text, schema_json, entities_json, enabled, created_at, updated_at` — no `written_by` column.
- `PolicyRow` Rust struct (`db/mod.rs:1356`): matches above — no `written_by`.
- `list_policies()` query (`db/mod.rs:1069`): `SELECT id, policy_text, schema_json, entities_json, enabled FROM authz_policies` — no join to `cedar_policy_versions`.
- `written_by` lives only on `cedar_policy_versions` rows. The latest version's `written_by` is the last modifier.
- The TypeScript `PolicyRow` interface (`web/src/api/types.ts:57`) has no `written_by` field.
- `PolicyTableRow` component (`Policies.tsx:331`) renders `ID | Policy (truncated) | Status | Actions` — no `written_by` column.

Two implementation options:

**Option A (JOIN — no schema change, recommended)**: Modify `list_policies()` to LEFT JOIN `cedar_policy_versions` on `policy_id = id AND version_num = (SELECT MAX(version_num) FROM cedar_policy_versions WHERE policy_id = authz_policies.id)`. Return `written_by` from the latest version row. Add `written_by: Option<String>` to `PolicyRow` Rust struct. Add `written_by?: string | null` to TS `PolicyRow`. Add a column to `PolicyTableRow`.

**Option B (denormalized column on `authz_policies`)**: Add `written_by TEXT` to `authz_policies` and update it on every upsert. Simpler query, but schema migration required and denormalized.

Recommendation: Option A. No migration required — `cedar_policy_versions` already has the data. The correlated subquery is acceptable for the small number of policies in a typical deployment.

---

## Summary of Gaps

| Goal | Gap Type | Effort |
|------|----------|--------|
| G1: Diff view | Full — no diff logic or library | Medium (library + UI component) |
| G2: Approvals poll | Small — `refetch` exists but unused | Trivial (~10 lines in `Approvals.tsx`) |
| G3: History pagination | Partial — API ready, UI has no pagination state | Small (~40 lines in `PolicyVersionHistory`) |
| G4: `written_by` in list | Backend + frontend — JOIN query + TS type + UI column | Medium (Rust + TS + JSX) |

---

## Starting Point (Verified)

- **Workspace health**: 560 tests passing, clippy clean (`-D warnings`), `tsc --noEmit` clean, `vite build` clean.
- **`fetchPolicyHistory` API**: exists and pagination-capable.
- **`useApprovals().refetch`**: available, just not called on a timer.
- **`cedar_policy_versions.written_by`**: populated in DB; accessible via JOIN.
- **No diff library installed**: needs to be added (`diff@^7` recommended, ~15 KB gzipped).

---

## Open Questions for Plan/Spec

1. **Diff display format**: unified diff (inline `+/-` lines, one pane) or side-by-side (two panes)? Unified is far simpler to implement with the `diff` library output. Recommended: start with unified, side-by-side can come later.
2. **Which two versions to diff**: should the operator select both explicitly (checkbox on two rows), or always diff "selected vs. previous" (N vs. N-1)? Simpler: always diff selected version vs. immediately prior version (N vs. N-1). The operator can navigate to other versions by clicking View.
3. **`written_by` attribution in upsert**: the upsert handler currently passes `None` for `written_by` (comment in `db/mod.rs:1109` says "caller identity wiring is deferred"). Should this phase wire the JWT `sub` claim into upsert calls? This affects G4 usefulness — without it, `written_by` is always `null` for new writes. **Recommend: yes, wire it in this phase** as part of the G4 change. The JWT middleware already extracts the subject; it just needs to be passed into `upsert_policy_inner`.
4. **History panel diff vs. view mode**: should the existing "View" read-only pane become the diff pane, or should "View" and "Diff" be separate modes? Recommended: replace "View" with a toggle between "Text" (current read-only textarea) and "Diff" (diff vs. prior version). This avoids adding a third column to the version table.

---

## Recommended Plan Order

1. **G2: Approvals auto-polling** — trivial, highest user-safety impact (approvals have timeouts), no dependencies.
2. **G4: `written_by` in policy list** — backend (Rust JOIN + TS type) + frontend (column), no external deps.
3. **G3: History pagination** — UI-only change, no backend work.
4. **G1: Policy diff view** — new dependency + most UI work; delivers last as the polish cap.
