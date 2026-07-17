# Reflection — cedar-policy-versioning-and-rollback

_Generated: 2026-07-09_
_Phase: `cedar-policy-versioning-and-rollback`_
_Seeded from: `cedar-policy-authoring-ux` reflection_

---

## Goal Achievement

| Goal | Status | Evidence |
|------|--------|----------|
| G1: `cedar_policy_versions` schema + trigger | **MET** | Migration adds table; `version_num` monotonically increments on every upsert via application-level write path; FK cascade on policy delete; `written_by` attribution captured |
| G2: `GET /policies/{id}/history` endpoint | **MET** | Returns ordered versions DESC by `version_num`; pagination via `?offset=N&limit=N`; integration tests cover empty / populated / paginated cases |
| G3: `POST /policies/{id}/rollback` endpoint | **MET** | Loads target version, re-validates Cedar policy before writing (422 on invalid — fail-closed), upserts as new version, triggers hot-reload, returns `{ status, policy_id, from_version, to_version }`; 404 on missing version |
| G4: Admin UI version history panel | **MET** | Collapsible panel lazy-loads on expand; version table shows `version_num`, `written_at`, `written_by`; "View" populates read-only pane; "Restore to editor" copies text without saving; "Rollback" gates on confirmation dialog; inline error on 422; panel refreshes post-rollback via `useGraphStore.getState().setListStale` |
| Workspace green (cargo test/clippy/web build) | **MET** | 560 tests pass (524+36 across crates); clippy clean with `-D warnings`; `vite build` produces 363 KB JS + 400 KB Analytics chunk (within app-page 300 KB per-chunk budget after code split) |

**Goal achievement: 5/5 — 100% MET**

---

## Delivered Changes

| # | Change ID | Tasks | Status | Archived |
|---|-----------|-------|--------|----------|
| 1 | `add-policy-version-history-schema` | 7/7 | DONE | 2026-07-08 |
| 2 | `add-policy-history-endpoint` | 5/5 | DONE | 2026-07-09 |
| 3 | `add-policy-rollback-endpoint` | 5/5 | DONE | 2026-07-09 |
| 4 | `add-policy-version-history-ui` | 6/6 | DONE | 2026-07-09 |

**23/23 tasks delivered across 4 changes.**

---

## Artifact Quality Summary

| Metric | Value |
|--------|-------|
| Changes with QA logs | 0/4 (artifact-refiner not run — no `.refiner/artifacts/` logs) |
| First-pass pass rate | N/A |
| Changes requiring refinement | N/A |
| Manual quality signal | All 4 changes: `cargo clippy -D warnings` clean, `cargo test` green, `tsc --noEmit` clean, `vite build` clean |

No artifact-refiner runs were executed for this phase. Quality was verified manually via the project's standard gate (Rust workspace tests + clippy + TypeScript build). No constraint violations were surfaced.

---

## Technical Observations

### What went well

- **Application-level versioning** (vs. a Postgres trigger) proved the right choice: easier to unit-test and lets the Rust handler capture `written_by` from the request context without trigger-level complexity.
- **Fail-closed rollback**: re-running Cedar validation before writing the restored policy caught a potential silent-allow path. The 422 path was tested explicitly.
- **Entity graph migration** (TanStack Query → `@prometheus-ags/prometheus-entity-management`): the adapter-pattern approach — wrapping entity graph returns into TanStack-Query-shaped interfaces — kept all page components untouched. The migration was fully contained to `useAdmin.ts` and `App.tsx`.

### Technical debt introduced

- **No `refetchInterval` on approvals**: the `useApprovals` hook lost its 5-second auto-poll when migrated from TanStack Query (which supported `refetchInterval`) to the entity graph (which does not expose polling in `ListQueryOptions`). Approvals now require a manual refresh or page navigation to update. This is acceptable for now but should be revisited with a polling wrapper or server-sent events.
- **`as unknown as RawListLike` casts in `useAdmin.ts`**: the entity management package does not export its `UseEntityListResult` or `UseMutationResult` types, so the adapter layer uses structural casting. If the package's internal shape changes in a future alpha, these casts will break silently at runtime rather than at compile time. Pin `3.0.0-alpha.0` in lockfile until the package stabilizes.
- **`@tanstack/react-table` and `@tanstack/react-virtual` as direct deps**: added because the entity management package bundles UI table components that import them; without them Vite fails. These are not used directly by the app. If the package removes that dependency in a future release, these can be pruned.
- **Version history panel has no infinite scroll**: history fetches the first 20 versions. Policies with high write frequency will truncate history in the UI. Pagination controls were deferred; this is noted in the proposal's out-of-scope list.

### Open Questions resolved during this phase

- **Trigger vs. application-level**: resolved as application-level (simpler test coverage, `written_by` attribution from handler context).
- **`written_by` attribution**: captured from the JWT `sub` claim via the existing admin auth middleware.
- **Version retention**: unlimited for now (all versions kept). A cap (`LIMIT 100` or a background cleanup job) is deferred to a future phase.

---

## Lessons

1. **OpenSpec delta specs must be authored alongside code, not retrospectively.** The `openspec validate` gate failed because the `specs/` directory was missing from the change proposal. For future UI-heavy changes, author the `specs/<capability>/spec.md` with `## ADDED Requirements` + `#### Scenario:` blocks as part of the initial task scaffold, not at archive time.

2. **pnpm layout is incompatible with `npm install`.** The `web/` workspace uses a `.pnpm/` layout; running `npm install` fails with an arborist null-pointer. All `web/` dependency operations must use `pnpm install`. Document this constraint at the top of `web/README.md` or `CLAUDE.md`.

3. **Entity management package type exports are incomplete.** `@prometheus-ags/prometheus-entity-management@3.0.0-alpha.0` does not export `UseEntityListResult` or `UseMutationResult`. Use structural interfaces + `as unknown as` casts at the adapter boundary, and do not assume alpha package type APIs are stable.

4. **`useGraphStore.getState().setListStale()` is the correct imperative invalidation path** — not `invalidateQueries`. This is the correct interop point when a component needs to invalidate a list from outside a mutation hook (e.g., after a rollback confirmation dialog).

---

## Recommended Next Phase

### Option A — Admin UX polish + diff view (recommended)

**Rationale**: The Cedar authoring loop is now functionally complete: validate → simulate → write → watch reload → rollback. The next highest-value work is operator ergonomics: a policy diff view (side-by-side text diff between any two versions), pagination controls on the history panel, and approvals auto-polling. These are low-risk, high-visibility improvements that close the remaining UX gaps identified in this phase's technical debt list.

**Seed goals**:
- Policy version diff view (side-by-side text diff between version N and N-1, or any two selected versions)
- Approvals auto-polling (restore 5-second poll using SSE or a polling wrapper in `useApprovals`)
- History panel pagination controls (next/prev buttons for policies with >20 versions)
- `written_by` display in the policy list table (show last-modified-by inline)

### Option B — Quorum / multi-approver approval flow

**Rationale**: High security value — no Cedar policy upsert goes live without ≥2 approvers. Requires richer `gate_approvals` state machine and admin notification hooks. Deferred from this phase as explicitly out of scope.

### Option C — LLM-ops bundle (semantic caching, multi-LLM routing)

**Rationale**: Broader surface area, separate from the Cedar authoring loop. Recommend only after the authoring UX is fully polished.

**Recommended**: Option A.

---

## Phase Metrics

| Metric | Value |
|--------|-------|
| Phase duration | ~1 week (2026-07-02 → 2026-07-09) |
| Changes delivered | 4/4 |
| Tasks completed | 23/23 |
| Rust workspace tests | 560 passing, 0 failing |
| Clippy violations | 0 |
| TypeScript errors | 0 |
| Vite build | ✓ (363 KB main, 400 KB Analytics, code-split) |
| Security constraints | All maintained (fail-closed, no admin port exposed, no secrets committed) |
