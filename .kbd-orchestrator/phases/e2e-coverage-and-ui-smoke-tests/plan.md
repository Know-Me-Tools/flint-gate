# Plan — e2e-coverage-and-ui-smoke-tests

_Generated: 2026-07-09_

## Change Backend

OpenSpec (`openspec/` directory present at project root).

## Ordered Changes

| Order | Change ID | Depends On | Scope | Agent |
|-------|-----------|------------|-------|-------|
| 1 | `add-playwright-webserver-config` | — | `web/playwright.config.ts` | claude-code |
| 2 | `add-policies-ui-smoke-tests` | #1 | `web/e2e/policies.spec.ts` (new) | claude-code |
| 3 | `add-approvals-smoke-test` | #1 | `web/e2e/approvals.spec.ts` (new) | claude-code |
| 4 | `add-e2e-ci-workflow` | #2, #3 | `.github/workflows/e2e-smoke.yml` (new) | claude-code |

## Ordering Rationale

Change #1 (`add-playwright-webserver-config`) is a strict prerequisite for everything else:
the `webServer` block is what makes `pnpm test:e2e` self-contained. Without it, CI cannot run
the tests. Changes #2 and #3 are independent of each other (different spec files, different
page routes) but both require #1. Change #4 requires #2 and #3 to be passing before the CI
workflow is meaningful.

## First Change to Apply

`/kbd-apply add-playwright-webserver-config`

## No External Library Research Needed

All tooling is already installed (`@playwright/test ^1.61.1`). The `page.clock` API is
available at that version. No new npm dependencies required.

## Out of Scope (confirmed from assessment)

- Full accessibility audit
- Visual regression screenshots
- Backend API integration tests
- New feature development
- "Create policy → appears in list" regression test (requires real DB or complex mock chain)
