# fix-admin-ui-readonly-indicators

**Phase:** beta-release-readiness / Phase 3 (Medium gap M-3)

## Problem

The admin UI has pages for managing `AuthProviders`, `Hooks`, and `Budgets`
that are read-only views (data is displayed but edits go through the API).
There is no visual indicator informing the operator that these pages are
read-only. Beta customers who attempt to edit data in the UI will receive
confusing no-op behavior or silent failures.

## Solution

Add a `ReadOnlyBanner` component that renders a visible, non-dismissible
notice at the top of read-only pages:

```
⚠️  This page is read-only. Use the API to make changes.
```

Apply `ReadOnlyBanner` to:
- `AuthProviders.tsx` (or equivalent)
- `Hooks.tsx` (or equivalent)
- `Budgets.tsx` (or equivalent)

The banner should use the existing design system colors and not break the
page layout. It should link to the relevant API docs section.

## Files to change

- `admin-ui/src/components/ReadOnlyBanner.tsx` (new) — reusable banner component
- `admin-ui/src/pages/AuthProviders.tsx` — import and render banner
- `admin-ui/src/pages/Hooks.tsx` — import and render banner
- `admin-ui/src/pages/Budgets.tsx` — import and render banner
- `admin-ui/src/components/ReadOnlyBanner.test.tsx` (new) — basic render test

Note: actual file paths in `admin-ui/src/` may differ — read the current
directory structure before applying and adjust accordingly.
