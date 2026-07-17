# Assessment — e2e-coverage-and-ui-smoke-tests

_Date: 2026-07-09_

## Executive Summary

The Playwright scaffold is solid and the test runner is wired. Two spec files exist:
`web/e2e/smoke.spec.ts` (3 UI tests against a live dev stack) and `web/e2e/oauth.spec.ts`
(API-level OAuth tests, skipped unless `E2E_OAUTH=1`). Neither spec covers the 5 UI surfaces
delivered in `admin-ux-polish-and-diff-view`. There is also no CI job that runs Playwright.

Gap: **5 UI features, 0 Playwright tests, no CI gate.**

---

## Existing Test Infrastructure

### Playwright setup

| Item | State |
|------|-------|
| `@playwright/test` | `^1.61.1` — installed |
| `playwright.config.ts` | chromium only, `baseURL: http://localhost:5173`, `testDir: ./e2e` |
| `pnpm test:e2e` script | wired → `playwright test` |
| `web/e2e/smoke.spec.ts` | 3 tests: nav bar, Routes page, API Keys page |
| `web/e2e/oauth.spec.ts` | API-level OAuth tests, gated behind `E2E_OAUTH=1` |
| Test pattern | Live Vite dev server + real admin API (no API mocking) |
| CI job for Playwright | **ABSENT** — `ci.yml` has Rust + Docker only |

### Admin API mock / fixture strategy

The existing smoke tests hit the admin API at `http://127.0.0.1:4457` via Vite's `/api` proxy.
They rely on the server returning empty state (`No routes configured.`). This means:
- Tests require the admin server to be running
- Playwright `page.route()` interceptor pattern is **not yet established** in this project
- New tests for Policies/Approvals will need either (a) a running admin server with seeded
  data, or (b) `page.route()` mocks to return fixture JSON

Using `page.route()` mocks is the right choice for the 5 new features: they exercise UI
rendering logic without requiring policy data to pre-exist in a real DB.

---

## Gap Analysis by Goal

### Goal 1: Policies page — "Last by" column

**What was shipped:**
- `<TableHead>Last by</TableHead>` in the Policies table header (line 153)
- `<TableCell>{policy.written_by ?? '—'}</TableCell>` per row (line 358)
- API response includes `written_by?: string | null` from `LEFT JOIN LATERAL` on
  `cedar_policy_versions`

**What's missing:**
- No E2E test navigates to `/policies` and asserts the "Last by" column header renders
- No test checks that the `—` placeholder appears when `written_by` is null
- No test checks that a real user ID appears when `written_by` is populated

**Test scope needed:**
1. Mock `GET /api/policies` → return one policy with `written_by: 'user-123'` and one with
   `written_by: null`
2. Navigate to `/policies`, assert "Last by" header visible
3. Assert `user-123` visible in the row
4. Assert `—` visible in the null row

---

### Goal 2: Policies page — history panel "Load more"

**What was shipped:**
- `hasMore` state computed from `total_hint` vs `versions.length`
- "Load more" button (lines 596–610) conditionally rendered when `hasMore === true`
- `loadMoreHistory()` appends next page at offset `offset + PAGE`

**What's missing:**
- No E2E test opens the version history panel on any policy
- No test asserts "Load more" button appears when `total_hint > versions.length`
- No test clicks "Load more" and verifies additional rows append

**Test scope needed:**
1. Mock `GET /api/policies` → one policy
2. Mock `GET /api/policies/{id}/history` → `{ versions: [20 rows], total_hint: 25 }`
3. Open history panel, assert 20 rows visible, assert "Load more" button visible
4. Click "Load more", mock second page → assert 5 more rows appended, "Load more" gone

---

### Goal 3: Policies page — "Text / Diff" toggle + PolicyDiffView

**What was shipped:**
- `viewMode: 'text' | 'diff'` state (line 438)
- Toggle buttons shown only when `viewedVersion.version_num > 1` (lines 620–636)
- `PolicyDiffView` component renders unified diff via `createPatch` (lines 380–419)
- Color-coded lines: `+` = green, `-` = red, `@` = muted

**What's missing:**
- No E2E test clicks a version row to set `viewedVersion`
- No test verifies the "Text / Diff" toggle appears for version > 1 but not version 1
- No test switches to Diff mode and asserts the diff output renders

**Test scope needed:**
1. Mock history to return 2 versions (v1 + v2 with differing `policy_text`)
2. Click version row for v2 → assert "Text" + "Diff" toggle visible
3. Click version row for v1 → assert toggle hidden (only one version, nothing to diff)
4. On v2 with toggle visible: click "Diff", assert `<pre>` with `+` / `-` lines renders

---

### Goal 4: Approvals page — auto-polling

**What was shipped:**
- `useEffect` + `setInterval(() => { refetch(); }, 5_000)` in `Approvals.tsx` (lines 90–93)
- Interval cleared on unmount via returned teardown

**What's missing:**
- No E2E test navigates to `/approvals` and verifies the page loads
- No test for the polling behavior (timer-based, hard to test deterministically in E2E)
- Existing smoke test does **not** include `/approvals` navigation

**Test scope needed:**
1. Navigate to `/approvals`, assert "Pending Approvals" heading visible
2. Mock `GET /api/approvals` → empty list, assert "No pending approvals." empty state
3. For polling: use `page.clock.install()` (Playwright fake timers, available since v1.45)
   to advance 5 seconds and verify a second `/api/approvals` request fires

---

### Goal 5 (Regression): Existing Policies golden path

The prior smoke test covers Routes and API Keys. No existing test covers Policies at all.

**Test scope needed:**
1. Navigate to `/policies`, assert "Policies" heading visible with empty state
2. (Optional stretch) Create policy via UI form and assert it appears in the list — this
   requires a real server or a more involved mock sequence; defer to a future phase.

---

## CI Gap

No `e2e-smoke.yml` GitHub Actions workflow exists. The `ci.yml` job covers Rust + Docker only.
A new workflow is needed that:
- Starts the Vite dev server in background
- Runs `playwright test --project chromium` (subset: skip OAuth tests)
- Uploads Playwright traces/report as artifacts on failure

The workflow can use `npx playwright install chromium` to avoid a full browser matrix.
`webServer` config in `playwright.config.ts` can auto-start Vite before the test run —
removing the manual setup step from CI.

---

## What Already Works (No Changes Needed)

- `playwright.config.ts` — correct; add `webServer` block and that's it
- `web/e2e/smoke.spec.ts` — passing tests, keep intact
- `web/e2e/oauth.spec.ts` — correctly gated behind `E2E_OAUTH=1`
- `pnpm test:e2e` script — correct
- All new UI components (`PolicyDiffView`, "Load more", "Last by", Approvals polling) — code
  is correct; this phase only adds tests

---

## Recommended Change Set (4 changes, no backend work)

| # | Change ID | Scope | Files |
|---|-----------|-------|-------|
| 1 | `add-playwright-webserver-config` | Add `webServer` to `playwright.config.ts` | `web/playwright.config.ts` |
| 2 | `add-policies-ui-smoke-tests` | Playwright tests for "Last by", "Load more", diff toggle | `web/e2e/policies.spec.ts` (new) |
| 3 | `add-approvals-smoke-test` | Playwright tests for Approvals load + polling via fake timers | `web/e2e/approvals.spec.ts` (new) |
| 4 | `add-e2e-ci-workflow` | GitHub Actions workflow running Playwright on PRs | `.github/workflows/e2e-smoke.yml` (new) |

### Ordering rationale

1 must precede 2/3 (webServer config makes local `pnpm test:e2e` self-contained).
2 and 3 can be done in parallel.
4 depends on 2 and 3 passing locally.

---

## Open Questions for Plan

1. **API mock strategy** — `page.route()` interceptors (no external dependency) vs. MSW
   (would require adding a dev dependency). Recommendation: `page.route()` — already
   available in Playwright, zero new deps.

2. **Fake timers for polling** — Playwright `page.clock.install()` / `page.clock.tick()`
   was added in v1.45; the project uses `^1.61.1`, so it's available. This is the right
   approach for the auto-polling test.

3. **CI admin server** — The webServer Vite proxy points at `http://127.0.0.1:4457`. In CI
   there is no real admin server. Tests using `page.route()` mocks don't need one; tests
   NOT using mocks will fail if the admin API is absent. New tests will all use mocks.
   The existing `smoke.spec.ts` tests expect empty API responses (which 404 or return empty
   JSON when the admin server is absent) — need to verify they still pass in CI once
   `webServer` is wired.

4. **Scope of regression suite** — "Create policy → appears in list" requires either a
   running admin server or a complex mock sequence (POST then GET returning the new row).
   Assessment: out of scope for this phase, deferred.
