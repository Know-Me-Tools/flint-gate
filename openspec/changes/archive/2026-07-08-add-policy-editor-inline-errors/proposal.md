# Proposal — add-policy-editor-inline-errors

**Phase:** cedar-policy-authoring-ux
**Goal:** G3 (MEDIUM)
**Build position:** 3 of 4 — depends on `POST /policies/validate` from change 1.

## Problem

The admin UI policy editor (`web/src/pages/Policies.tsx`) is a plain `<Textarea>` that shows a transient toast notification on save error. Cedar parse errors from the server are a flat string in the toast description. There is no live validation, no inline error markers, no line/column annotation, and no way to validate without saving.

## Solution

1. Add `POST /policies/validate` API call to `web/src/api/admin.ts`.
2. Add debounced validate-on-change to `PolicyForm` (500ms debounce via `useEffect` + `useRef` for the timeout handle).
3. Render `errors[]` from the validate response as an inline error list below the Cedar textarea: `Line N, Col M: <message>` for each entry.
4. Disable the Save button when the validate response has `valid === false`.
5. Add an explicit "Validate" button for on-demand validation without saving.
6. Show a green checkmark / "Policy is valid" indicator when `valid === true`.
7. Wire the admin `GET /events` SSE endpoint (change 2) so hot-reload errors appear in a notification area on the Policies page.

## Scope

- `web/src/api/admin.ts` — `validatePolicy(text: string, schema?: string): Promise<ValidateResponse>` API call
- `web/src/pages/Policies.tsx` — debounced validate hook; inline error list; Save button disabled state; Validate button; hot-reload error notification via EventSource
- `web/src/types/admin.ts` (or similar) — `PolicyParseError`, `ValidateResponse` TypeScript types

## Acceptance criteria

- Typing invalid Cedar syntax into the policy editor shows `Line N, Col M: <message>` below the textarea within ~500ms of stopping typing (debounce).
- The Save button is disabled while the policy is invalid; it re-enables when the policy becomes valid.
- Clicking the standalone "Validate" button triggers immediate validation (not debounced).
- A valid policy shows a "Policy is valid" indicator (no errors list shown).
- Hot-reload error events from `GET /events` SSE appear as a dismissable warning banner on the Policies page.
- Existing save behavior (POST /policies, PUT /policies/{id}) is unchanged.
- TypeScript compiles clean (`tsc --noEmit`); no `any` types in new code.
