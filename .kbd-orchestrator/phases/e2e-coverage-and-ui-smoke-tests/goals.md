# Goals — e2e-coverage-and-ui-smoke-tests

_Seeded from `admin-ux-polish-and-diff-view/reflection.md` → "Recommended Next Phase → Option A"._

The Cedar authoring loop and admin UI are feature-complete through the previous phase. This phase
closes the Playwright E2E gap: the scaffold was committed (`cc0626d`) but no tests exist for the
new UI surfaces shipped since then.

1. **Smoke tests for new Policies UI surfaces** — Playwright tests covering:
   - "Text / Diff" toggle in the version history panel renders a unified diff for version > 1
   - "Load more" button appears when `hasMore` is true and appends additional version rows
   - "Last by" column in the Policies table shows the `written_by` value (or `—` when absent)

2. **Approvals auto-polling smoke test** — verify the Approvals page re-fetches without user
   interaction (assert network calls or use a clock mock in Playwright).

3. **Regression guard on existing Policies and Approvals pages** — key golden-path flows must
   pass:
   - Create policy → appears in list
   - Enable/disable policy toggle
   - Open version history panel
   - Approval queue renders (with empty-state)

4. **CI gate** — all E2E tests must pass in the GitHub Actions smoke workflow
   (`e2e-smoke.yml`) before the phase is closed.

## Success Criteria

- [ ] Playwright test file(s) exist under `e2e/` (or wherever the scaffold placed them) covering
      the 3 new Policies surfaces and the Approvals polling flow.
- [ ] Regression suite covers create-policy, toggle, and history-panel open.
- [ ] `pnpm test:e2e` (or equivalent) exits 0 locally with the dev stack running.
- [ ] CI workflow `e2e-smoke.yml` passes on the branch.

## Explicitly Out of Scope

- Full accessibility audit (defer to a dedicated a11y phase).
- Visual regression screenshots (defer unless a Playwright plugin is already wired).
- Backend API integration tests (covered by Rust `#[test]` suite).
- New feature development.
