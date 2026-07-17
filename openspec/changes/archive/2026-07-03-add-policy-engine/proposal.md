# add-policy-engine

## Summary
Add an embedded, runtime-editable authorization policy engine (Cedar) with route-level decisions, hot-reload, and write-time validation. (Goal G2a)

## Design
New `authz/` module wrapping Cedar: compiled `PolicySet` + `Schema` + `Entities` shared lock-free via `arc_swap::ArcSwap<Arc<CedarBundle>>`. Store policy text + schema in a new `authz_policies` Postgres table (JSONB) applied via the idempotent `migrate()` pattern. Hot-reload = parse-before-swap, **fail-closed** (retain last-good bundle on parse/validation error). Validate stored policies against the schema at write-time in the Admin API via Cedar `Validator` so bad policy never reaches the hot path. Add a `PreRequestHook::Authorize` variant for route-level decisions. Model actions generically (`call_tool` / route action) with attributes in `context` to avoid schema churn.

Library: adopt `cedar-policy` 4 + `arc-swap` 1 (library-candidates.json G2). Fallback regorus only if a policy need exceeds Cedar's language.

## Depends on
- add-mcp-resource-server (identity claims plumbed as Cedar principal/context)

## Scope
IN: Cedar engine wrapper, ArcSwap hot-reload (fail-closed), authz_policies table, Admin API policy CRUD + write-time validation, `Authorize` pre-request hook. OUT: per-tool-call stream gating (add-per-tool-authz), audit trail (add-authz-audit-trail).

## Tasks
- [ ] Add `cedar-policy = "4"` + `arc-swap = "1"` to Cargo.toml
- [ ] `authz/` module: CedarBundle (PolicySet + Schema + Entities), ArcSwap sharing
- [ ] `authz_policies` table via `migrate()`; load policy text + schema from JSONB
- [ ] Hot-reload: parse-before-swap, fail-closed (retain last-good)
- [ ] Admin API: policy CRUD + write-time Cedar `Validator` (reject invalid before store)
- [ ] `PreRequestHook::Authorize` variant → route-level decision in pipeline
- [ ] Tests: allow/deny, bad-policy fail-closed, write-time rejection; ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
