# add-cedar-entity-schema

**Phase:** beta-release-readiness / Phase 2 (Serious gap S-4)

## Problem

Policies are stored with `schema_json: None`. Cedar validates syntax only.
A typo like `@require_apporval` (wrong annotation spelling) is accepted silently
and the policy behaves as a plain permit — no approval gate fires.

## Solution

Define the gateway's Cedar entity schema as a constant in `authz/` and enforce
it at policy write time:

- Entity types: `User`, `Agent`, `Service`, `Route`
- Actions: `Action::"call_tool"` (the only valid action in this domain)
- Annotation: `@require_approval(String)` is a known, valid annotation
- Enforce via `validate_policy`, `create_policy_handler`, and `update_policy_handler`

Policies that reference undefined entity types, use undefined actions, or use
misspelled annotations must return HTTP 422 with a descriptive error.

## Files to change

- `crates/flint-gate-core/src/authz/engine.rs` or new `authz/schema.rs`
- `crates/flint-gate-core/src/authz/bundle.rs` — populate `schema_json` in PolicyRecord
- `crates/flint-gate-core/src/admin/mod.rs` — validate against schema in create/update/validate handlers
- `crates/flint-gate-core/src/db/mod.rs` — schema_json column should be populated for gateway-native policies
