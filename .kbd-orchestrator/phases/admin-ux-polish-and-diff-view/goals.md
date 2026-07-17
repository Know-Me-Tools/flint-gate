# Goals — admin-ux-polish-and-diff-view

_Seeded from `cedar-policy-versioning-and-rollback/reflection.md` → "Recommended Next Phase → Option A"._

The Cedar authoring loop is functionally complete. This phase closes the remaining operator UX gaps identified in the prior phase reflection:

1. **Policy version diff view** — side-by-side text diff between any two selected versions (e.g., current vs. N-1, or any pair). Makes rollback intent observable before confirming.

2. **Approvals auto-polling** — restore the 5-second refresh on the Approvals page that was lost when migrating from TanStack Query (which had `refetchInterval`) to `@prometheus-ags/prometheus-entity-management`. Use SSE, a polling wrapper, or `setListStale` on a timer.

3. **History panel pagination** — "Load more" / next-prev controls on the version history panel for policies with > 20 versions.

4. **`written_by` in policy list table** — surface last-modified-by inline in the `Policies` page table so operators can see authorship at a glance without expanding the editor.

## Success Criteria (draft — refine during Assess/Spec)

- [ ] Policy editor shows a diff pane when operator selects two history versions to compare.
- [ ] Approvals list auto-refreshes at ≤ 5-second intervals without manual page interaction.
- [ ] History panel supports fetching pages beyond the first 20 versions via explicit user action.
- [ ] Policy list table includes a `written_by` column (or similar) showing the last modifier.
- [ ] All new UI additions: `tsc --noEmit` clean, `vite build` passes, no new TS `any` without justification.

## Explicitly Out of Scope

- Multi-approver / quorum approval flow.
- Step-up authentication.
- Global point-in-time policy snapshot rollback.
- LLM-ops bundle (semantic caching, multi-LLM routing).
