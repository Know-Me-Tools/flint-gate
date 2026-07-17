- [x] Read `admin-ui/src/` directory tree to confirm actual page file names and component structure
      FINDING: Admin UI is at `web/src/` (not `admin-ui/src/`). Pages: AuthProviders.tsx, Hooks.tsx, Budgets.tsx confirmed.
- [x] Create `web/src/components/ReadOnlyBanner.tsx` with a styled non-dismissible notice
      Uses Tailwind yellow-toned warning design, accessible with role="status" aria-live, supports optional docsHref.
- [x] Use existing design system tokens (colors, spacing) from the project's CSS/Tailwind config
- [x] Import and render `<ReadOnlyBanner />` at the top of `AuthProviders.tsx`
- [x] Import and render `<ReadOnlyBanner />` at the top of `Hooks.tsx`
- [x] Import and render `<ReadOnlyBanner />` at the top of `Budgets.tsx`
- [x] Create `web/src/components/ReadOnlyBanner.test.tsx` with a render smoke test
      Type-level test (no unit runner installed); verified clean by `pnpm typecheck` / tsc --noEmit.
- [x] `pnpm typecheck` passes with no type errors
