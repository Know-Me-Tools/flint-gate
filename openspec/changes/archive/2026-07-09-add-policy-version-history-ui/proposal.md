# add-policy-version-history-ui

## Summary

Add a "Version History" collapsible panel to the admin policy editor in `Policies.tsx`. Operators can browse prior versions, view each version's policy text in a read-only pane, and restore any version with a confirmation-gated rollback button.

## Motivation

The history (`GET /policies/{id}/history`) and rollback (`POST /policies/{id}/rollback`) endpoints exist after changes 1–3. This change closes the authoring loop in the admin UI: validate → simulate → write → observe reload status → **inspect history → rollback**.

## Design

### New TypeScript types (`web/src/api/types.ts`)

```typescript
export interface PolicyVersionRow {
  id: number;
  policy_id: string;
  version_num: number;
  policy_text: string;
  schema_json?: unknown | null;
  entities_json?: unknown | null;
  written_by?: string | null;
  written_at: string;      // ISO-8601
}

export interface PolicyHistoryResponse {
  policy_id: string;
  total_hint: number | null;
  offset: number;
  limit: number;
  versions: PolicyVersionRow[];
}

export interface RollbackResponse {
  status: string;          // "rolled_back"
  policy_id: string;
  from_version: number;
  to_version: number;
  reloaded: boolean;
}
```

### New API functions (`web/src/api/admin.ts`)

```typescript
export async function fetchPolicyHistory(
  id: string,
  offset = 0,
  limit = 20,
): Promise<PolicyHistoryResponse>

export async function rollbackPolicy(
  id: string,
  versionNum: number,
): Promise<RollbackResponse>
```

### UI changes (`web/src/pages/Policies.tsx`)

Inside `PolicyForm` (the edit modal/drawer), add a collapsible `<details>` or accordion section below the policy textarea — "Version History". Behavior:

1. **Lazy load**: history is fetched only when the section is expanded (not on modal open), using a local `useState` + `useEffect` triggered by the open state.
2. **Version list**: each row shows `version_num`, `written_at` (formatted), `written_by` (if non-null), and two actions: "View" and "Rollback to this version".
3. **View action**: clicking "View" populates a read-only `<textarea>` below the list with the selected version's `policy_text`. A "Restore to editor" button copies the text into the editable textarea (without saving) so the operator can review and manually save.
4. **Rollback action**: clicking "Rollback to this version" shows a confirmation dialog (`window.confirm` or a simple inline confirm UI — match the existing design system). On confirm, calls `rollbackPolicy(id, version_num)`. On success: dismiss the dialog, refresh the policy list (invalidate TanStack Query), refresh the history panel, show a success toast/notification. On error: show the error message (including 422 Cedar validation errors).
5. **Loading and error states**: spinner while fetching, inline error message on failure.

### Design constraints

- Do not add heavy new dependencies. The history list is a simple table; no virtualization needed.
- The collapsible section should not shift layout when closed — use `<details>` with CSS or a simple `open` state toggle.
- Rollback confirmation must be visible and explicit — never a single-click action.

## Tasks
- [ ] Add `PolicyVersionRow`, `PolicyHistoryResponse`, `RollbackResponse` types to `web/src/api/types.ts`
- [ ] Add `fetchPolicyHistory(id, offset, limit)` and `rollbackPolicy(id, versionNum)` to `web/src/api/admin.ts`
- [ ] Add collapsible "Version History" section to `PolicyForm` in `Policies.tsx`: lazy-load on expand, version list with written_at + version_num, "View" and "Rollback" actions
- [ ] Implement rollback flow: confirmation gate → call `rollbackPolicy` → success refreshes policy list + history panel; error surfaces Cedar validation messages
- [ ] Implement "View" flow: populate read-only textarea with selected version's `policy_text`; "Restore to editor" button copies text to editable textarea
- [ ] TypeScript `--noEmit` clean; web `vite build` passes
