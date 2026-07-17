# Proposal — add-cedar-policy-validate-endpoint

**Phase:** cedar-policy-authoring-ux
**Goal:** G2 (HIGH)
**Build position:** 1 of 4 — foundation change; extracts structured Cedar error types used by all other changes.

## Problem

Cedar policy write-time validation is embedded in `upsert_policy_inner` as a side effect of saving. There is no dry-run validate endpoint. All Cedar parse errors are converted to flat strings at the call site (`.to_string()`), losing line/column information that Cedar 4.x's `ParseErrors` iterator provides via `SourceLocation`. Operators cannot validate a policy without persisting it.

## Solution

1. Define `PolicyParseError { line: usize, column: usize, length: usize, message: String }` in `authz/error.rs` (or a new `authz/parse_error.rs`).
2. Extract Cedar error structure: iterate `ParseErrors`, map each to `PolicyParseError` using `source_location()` byte offsets, derive line/column from the source text.
3. Extract `validate_policy_text(text: &str, schema: Option<&Schema>) -> Result<(), Vec<PolicyParseError>>` from `upsert_policy_inner`.
4. Add `POST /policies/validate` route and handler returning `{ valid: bool, errors: Vec<PolicyParseError> }`.
5. Rate-limited by the existing `AdminGovernorLayer` (no special casing).

## Scope

- `crates/flint-gate-core/src/authz/error.rs` — add `PolicyParseError` type
- `crates/flint-gate-core/src/authz/bundle.rs` — extract structured error instead of `.to_string()`
- `crates/flint-gate-core/src/authz/validator.rs` — same structured extraction
- `crates/flint-gate-core/src/admin/mod.rs` — add `POST /policies/validate` route + handler; refactor `upsert_policy_inner` to call the shared validate fn
- No changes to `main.rs`, no config changes, no UI changes in this change.

## Acceptance criteria

- `POST /policies/validate` with valid Cedar text returns `{ valid: true, errors: [] }` (200).
- `POST /policies/validate` with invalid Cedar text returns `{ valid: false, errors: [{ line, column, length, message }] }` (200 — not a 4xx; the validation result is the successful response).
- `errors[].line` and `errors[].column` are 1-based and match the actual position of the parse error in the submitted text.
- Existing `POST /policies` and `PUT /policies/{id}` continue to reject invalid policy with HTTP 422 (behavior unchanged; implementation now calls the shared validate fn).
- Rate-limited: returns 429 after burst exhaustion (same as other admin write endpoints).
- 3+ new unit tests covering: valid policy, parse error with line/col, schema validation error.
