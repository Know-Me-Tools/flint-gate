# Change: add-e2e-ci-workflow

## Summary

New GitHub Actions workflow `.github/workflows/e2e-smoke.yml` that installs Playwright
(chromium only), starts the Vite dev server via `webServer` config, and runs all non-OAuth
Playwright tests on every PR and push to the main branch.

## Motivation

No CI job runs Playwright. The 3 existing tests and the 8 new tests added in this phase only
run locally. Without a CI gate, regressions in the admin UI pass code review undetected.

## Design

```yaml
name: E2E Smoke

on:
  push:
    branches: [main, claude/flint-gate-auth-proxy-zQBD4]
  pull_request:
    branches: [main, claude/flint-gate-auth-proxy-zQBD4]

jobs:
  e2e:
    name: Playwright smoke tests
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: web
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: web/pnpm-lock.yaml
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
      - name: Install Playwright browsers (chromium only)
        run: pnpm playwright install chromium --with-deps
      - name: Run E2E smoke tests (skip OAuth)
        run: pnpm test:e2e --project chromium
        env:
          CI: true
      - name: Upload Playwright report
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: web/playwright-report/
          retention-days: 7
```

### Key decisions

- `--with-deps` installs system libraries needed by Chromium on Ubuntu
- OAuth tests auto-skip when `E2E_OAUTH` is unset (no Hydra in this job)
- `CI: true` in env triggers `reuseExistingServer: false` in `playwright.config.ts`
  and enables `forbidOnly` and `retries: 2` (already in the config)
- Playwright report uploaded as artifact on failure only (keeps passing runs lean)

## Why

No CI job runs Playwright. The 16 smoke tests added this phase only run locally, so any
regression in admin UI rendering, routing, or polling behaviour silently passes code review.
The workflow adds a mandatory gate on every PR.

## What Changes

- **`.github/workflows/e2e-smoke.yml`** (new) — 5-step CI job: checkout, pnpm + node
  setup, `pnpm install`, `playwright install chromium --with-deps`, `pnpm test:e2e`.
- **`web/e2e/smoke.spec.ts`** (modified) — admin-server-dependent tests wrapped with
  `skipWithoutServer()` so the CI job skips them cleanly rather than failing.

## Affected Files

- `.github/workflows/e2e-smoke.yml` (new)

## Verification

Create a draft PR; CI shows a passing `e2e / Playwright smoke tests` check.
