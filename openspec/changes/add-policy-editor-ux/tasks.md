# Tasks — add-policy-editor-ux

- [ ] Add CodeMirror 6 dependencies to `web/package.json` (`@codemirror/state`, `@codemirror/view`, `@codemirror/commands`, `@codemirror/language`)
- [ ] Create `web/src/components/CedarEditor.tsx` — CodeMirror 6 wrapper with Cedar keyword highlighting extension
- [ ] Replace `<Textarea>` in `PolicyForm` (inside `Policies.tsx`) with `<CedarEditor>`, preserving the `onChange` + validation wiring
- [ ] Confirm the existing debounced `validatePolicy()` call fires correctly on CodeMirror change events
- [ ] Read `Policies.tsx` policy list rendering — add approval count badge to each card that has `@require_approval` in its Cedar text
- [ ] Read `Approvals.tsx` to check for `?policy=` query param filter; add it if absent
- [ ] Wire badge click to navigate to `/approvals?policy=<id>`
- [ ] Run `pnpm typecheck` and confirm passing
