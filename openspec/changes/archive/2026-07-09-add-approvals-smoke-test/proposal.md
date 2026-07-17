# Change: add-approvals-smoke-test

## Summary

New Playwright spec `web/e2e/approvals.spec.ts` covering the Approvals page: page load, empty
state, pending approvals table, and the 5-second auto-polling behavior introduced in
`add-approvals-auto-polling`.

## Motivation

The Approvals page has no Playwright tests. The auto-polling `setInterval` was the primary
feature of the `add-approvals-auto-polling` change and has never been tested in a browser.

## Design

Uses `page.route()` for API mocking and `page.clock` for fake timer control (Playwright v1.45+;
project uses `^1.61.1`).

### Test cases

1. **Page load — empty state** — mock `GET /api/approvals` → `{ approvals: [] }`; navigate to
   `/approvals`; assert "Pending Approvals" heading visible; assert "No pending approvals." visible.

2. **Page load — approvals table renders** — mock `GET /api/approvals` → one pending approval row;
   navigate; assert "Approval ID" column header; assert the approval ID value visible in the table.

3. **Auto-polling fires after 5 seconds** — install `page.clock`; mock `/api/approvals` to count
   calls; navigate to `/approvals`; assert initial call (count = 1); advance clock by 5001 ms;
   assert second call fires (count = 2); advance by 5001 ms more; assert third call (count = 3).

4. **Navigation link visible from root** — navigate to `/`, assert "Approvals" link in the
   sidebar/nav; click it; assert "Pending Approvals" heading visible.

## Implementation Note on `page.clock`

```ts
await page.clock.install();
// ... navigate to /approvals ...
await page.clock.tick(5_001);
// assert refetch fired
```

`page.clock` replaces `Date`, `setTimeout`, and `setInterval` with controllable fakes.
`setInterval(() => { refetch(); }, 5_000)` in `Approvals.tsx` will tick on the fake clock.

## Why

The Approvals page has zero Playwright coverage. The 5-second auto-polling loop in `useApprovals()`
was the primary deliverable of `add-approvals-auto-polling` and has never been exercised in a
browser context. Shipping without a smoke test means regressions in the most-used real-time
behaviour go undetected.

## What Changes

- **`web/e2e/approvals.spec.ts`** (new) — 8 tests across 4 describe blocks: empty-state,
  table render, auto-polling (fake-clock), and navigation link.

## Affected Files

- `web/e2e/approvals.spec.ts` (new)

## Verification

`pnpm --dir web test:e2e --grep "Approvals"` exits 0.
