# Change: add-playwright-webserver-config

## Summary

Add a `webServer` block to `web/playwright.config.ts` so Playwright auto-starts the Vite dev
server before the test run. This makes `pnpm test:e2e` self-contained — no manual `pnpm dev`
required — and is the prerequisite for the CI workflow.

## Motivation

The existing smoke tests and all new Policies/Approvals tests assume `http://localhost:5173` is
serving the admin UI. Without `webServer`, the operator must start Vite manually before running
Playwright. The CI workflow cannot do that. Adding `webServer` eliminates the manual step.

## Design

```ts
webServer: {
  command: 'pnpm dev',
  url: 'http://localhost:5173',
  reuseExistingServer: !process.env.CI,
  timeout: 60_000,
},
```

- `reuseExistingServer: !process.env.CI` — reuses a running `pnpm dev` during local development
  (fast), but always starts fresh in CI (deterministic).
- `timeout: 60_000` — Vite cold start with full TypeScript compilation takes ~10 s; 60 s is
  generous but not runaway.

## Affected Files

- `web/playwright.config.ts` — add `webServer` block

## Verification

`pnpm --dir web test:e2e` exits 0 without a prior `pnpm dev` in another terminal.
