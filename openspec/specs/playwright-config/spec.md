# playwright-config Specification

## Purpose
TBD - created by archiving change add-playwright-webserver-config. Update Purpose after archive.
## Requirements
### Requirement: Playwright SHALL auto-start the Vite dev server before the test run

`web/playwright.config.ts` MUST configure a `webServer` block so `pnpm test:e2e` is
self-contained. In CI (`CI=true`) a fresh server SHALL always be started; locally an
existing dev server SHALL be reused.

#### Scenario: CI run starts Vite automatically

Given `CI=true` and no prior Vite process running,
when `pnpm test:e2e` is executed,
then Playwright starts `pnpm dev`, waits until `http://localhost:5173` is reachable,
and begins the test run without operator intervention.

#### Scenario: Local run reuses an existing dev server

Given a Vite dev server already running on port 5173,
when `pnpm test:e2e` is executed without `CI=true`,
then Playwright reuses the existing server and does not spawn a second one.

