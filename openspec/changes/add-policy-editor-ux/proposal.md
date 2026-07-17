# Proposal — add-policy-editor-ux

**Phase:** post-beta-hardening
**Goal:** G-3 — Admin UI Cedar policy editor UX gaps
**Severity:** LOW
**Depends on:** wire-postgres-approval-config (for stable approval IDs in linkage)

## Problem

`web/src/pages/Policies.tsx` is a complete policy management surface
(CRUD, inline validation, version history, rollback, tool scopes, SSE
hot-reload banner). Two UX gaps remain:

1. The policy textarea is a plain `<Textarea>` — no syntax highlighting
   for Cedar keywords, making complex policies error-prone to write.
2. Policies with `@require_approval` have no visual indication of pending
   approvals or a link to the Approvals page filtered to that policy.

## Scope

- `web/src/pages/Policies.tsx` — replace `<Textarea>` in `PolicyForm`
  with CodeMirror 6 editor; add approval count badge to policy cards
- `web/package.json` — add `@codemirror/state`, `@codemirror/view`,
  `@codemirror/lang-*` (minimal set for basic highlighting)
- `web/src/pages/Approvals.tsx` (read-only) — confirm `?policy=<id>`
  filter param is supported or add it

## Out of scope

- Full Cedar language server (LSP) integration
- Semantic validation in the editor (already handled by debounced API call)

## Acceptance Criteria

- Policy textarea renders with at minimum keyword highlighting for:
  `permit`, `forbid`, `when`, `unless`, `principal`, `action`, `resource`,
  `is`, `in`, `has`
- Existing inline validation (debounce + error list) continues to work
  in the CodeMirror editor
- Policy cards with `@require_approval` in the Cedar text show an
  "N pending" badge
- Clicking the badge navigates to `/approvals?policy=<id>`
- `pnpm typecheck` passes
- No existing functionality regresses (CRUD, version history, rollback)
