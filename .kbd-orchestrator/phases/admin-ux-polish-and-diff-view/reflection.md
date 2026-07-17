# Reflection — admin-ux-polish-and-diff-view

_Generated: 2026-07-09_

## Summary

All 4 stated goals were delivered in full within 5 changes (one bonus change added). Every change
is verified + archived in OpenSpec. The Cedar authoring loop now surfaces authorship, renders diffs,
pages through history, and keeps the approval queue live — the operator no longer needs to manually
refresh or blindly confirm rollbacks.

---

## Goal Achievement

| # | Goal | Status | Notes |
|---|------|--------|-------|
| 1 | Policy version diff view | **MET** | `PolicyDiffView` component + `createPatch` from `diff` library; "Text / Diff" toggle shown only when version > 1. Renders unified diff with green/red/muted coloring inline in the history panel. |
| 2 | Approvals auto-polling | **MET** | `useEffect` + `setInterval(refetch, 5_000)` in `Approvals.tsx`. Interval cleared on unmount. No SSE or external state required. |
| 3 | History panel pagination | **MET** | `loadMoreHistory()` + `PAGE = 20` constant; `hasMore` state derived from `total_hint`; "Load more" button shown when more versions exist. Offset advances correctly across pages. |
| 4 | `written_by` in policy list | **MET** | LEFT JOIN LATERAL on `cedar_policy_versions` returns the latest `written_by` without a migration to `authz_policies`. `Identity.id` from the admin auth middleware is threaded through to the INSERT. "Last by" column in the Policies table. |

**Bonus:** `add-sdk-policy-methods` — 7 new `FlintGateAdmin` methods (`listPolicies`, `getPolicy`, `createPolicy`, `updatePolicy`, `deletePolicy`, `getPolicyHistory`, `rollbackPolicy`) with 9 vitest unit tests. Not in the original plan but unblocks SDK consumers.

---

## Delivered Changes

| Change | Tasks | Archived |
|--------|-------|---------|
| `add-approvals-auto-polling` | 3/3 | 2026-07-09 |
| `add-written-by-to-policy-list` | 7/7 | 2026-07-09 |
| `add-history-panel-pagination` | 6/6 | 2026-07-09 |
| `add-policy-version-diff-view` | 7/7 | 2026-07-09 |
| `add-sdk-policy-methods` | 6/6 | 2026-07-09 |

---

## Technical Debt Introduced

1. **`diff` package type declarations** — `diff@7` ships no `.d.ts` files; `@types/diff@8` conflicts.
   Workaround: local `web/src/types/diff.d.ts` with a minimal `createPatch` declaration.
   Resolve: check periodically whether `diff` has shipped first-party types or `@types/diff` has
   aligned to v7. The local file is <10 lines and safe to remove when upstream improves.

2. **`written_by` read from `cedar_policy_versions`, not `authz_policies`** — the JOIN reads the
   last version row per policy on every `list_policies()` call. For tables with thousands of
   policies this adds a LATERAL per row. Index on `(policy_id, version_num DESC)` in
   `cedar_policy_versions` makes this fast, but it is worth revisiting with `EXPLAIN ANALYZE`
   before production load testing.

3. **Polling instead of SSE on Approvals** — a simple `setInterval` was the lowest-friction
   solution. For high-volume approval queues or mobile clients, replacing with an SSE subscription
   (the gate already has SSE infrastructure) would reduce unnecessary round-trips. Deferred to a
   future phase if queue volume justifies it.

4. **`total_hint` is null-permissive** — `getPolicyHistory` can return `total_hint: null`.
   The `computeHasMore` helper treats null as "unknown, show Load more". If the backend starts
   reliably supplying the count, tighten the logic.

5. **No Playwright coverage for new UI components** — `PolicyDiffView`, the "Load more" button,
   and the "Last by" column are not covered by the E2E scaffold committed in the prior phase. The
   scaffold exists; tests need to be authored.

---

## Lessons Captured

1. **`openspec archive` consumes only one piped `Y` when tasks are still marked incomplete** — the
   warning prompt appears *before* the spec-update confirmation prompt, consuming the first `Y`.
   Fix: always mark tasks done before archiving, or pipe `printf "Y\nY\n"`.

2. **The three-level spec hierarchy is mandatory for `openspec validate`** — `## ADDED Requirements`
   → `### Requirement:` → `#### Scenario:`. Omitting the `### Requirement:` level produces "no
   deltas found" silently. Validate after writing specs, not just before archiving.

3. **LEFT JOIN LATERAL is preferable to schema columns for audit metadata on a JOIN target** —
   adding `written_by` to `authz_policies` would have required a migration. Reading it from the
   latest `cedar_policy_versions` row is migration-free and keeps authorship in the version store
   where it belongs logically.

4. **`@types/diff` v8 conflicts with `diff` v7** — ambient declaration in the `@types` package
   registers the module globally; `diff@7`'s own types (if any) conflict. The right fix is a
   local `.d.ts`, not pinning `@types/diff` to an older version.

5. **`Option<Extension<Identity>>` is the correct Axum pattern for optional auth** — on
   loopback-dev deployments the middleware short-circuits without inserting the extension.
   Handlers must accept `Option<Extension<Identity>>` or they panic when the extension is absent.

---

## Artifact Quality Summary

QA gate ran via `openspec validate` + `openspec archive` after each change. No separate
artifact-refiner logs available (not wired for this phase). All 5 changes passed validate and
archive with zero open issues at archive time.

TypeScript build (`vite build`, `tsc --noEmit`) was verified clean after the diff-view change
resolved two `TS7006` implicit-any errors in the `.map` callback and removed the conflicting
`@types/diff` package.

---

## Recommended Next Phase

### Option A — E2E coverage catch-up + Playwright smoke tests

The E2E scaffold exists but the new UI surfaces (diff view, "Load more", "Last by" column) have
no coverage. One short phase to:
- Author Playwright tests for the 3 new Policies UI surfaces
- Author a test for Approvals auto-polling (mock timer or assert refetch count)
- Gate on CI passing

### Option B — Production hardening: rate-limit admin write endpoints + audit export

The `written_by` trail is now in place. Next logical step:
- Rate-limit `POST/PUT/DELETE /policies` on the admin port
- Add an audit-export endpoint (`GET /audit?entity=policy&limit=100`)
- Wire Cedar policy change events to the existing audit trail (`authz_audit_log`)

### Option C — LLM-ops bundle (semantic caching, multi-LLM routing)

Was explicitly out of scope for this phase. If business priority has shifted, this is the next
large feature surface.

**Recommendation: Option A** — the E2E gap is a near-term correctness risk. The new UI components
are entirely untested in the browser and the Playwright scaffold is already there. One focused phase
closes the gap before shipping the authoring loop to operators.
