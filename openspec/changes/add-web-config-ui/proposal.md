# add-web-config-ui

## Summary
Ship a web configuration + observability dashboard served by the Admin server: manage routes/auth/hooks/policies/budgets and view live token/cost + authz-audit analytics. (Goal G4)

## Design
A React + Vite SPA that imports the existing TypeScript SDK directly (stays in lockstep with the utoipa OpenAPI). Embed the built `dist/` into the binary via `rust-embed` 4→8 (`axum` + `debug-embed` features) served by the Admin server with SPA `index.html` fallback — preserves single-binary distribution (`ServeDir` cannot serve embedded assets). Add read-model Admin endpoints aggregating `usage_events` (token/cost analytics) and `authz_audit`. Charts via Recharts (cost/token); uPlot for dense time-series only. Code-split analytics to keep CRUD screens under the ~300 KB app-page JS budget.

Library: adopt `rust-embed` 8 (`axum`, `debug-embed`) + React/Vite + Recharts; reuse the existing TypeScript SDK (library-candidates.json G4).

## Depends on
- add-budget-rate-limiting, add-mcp-resource-server, add-policy-engine, add-per-tool-authz, add-authz-audit-trail, add-hitl-approval, add-guardrail-hook (surfaces what the UI manages) — built last

## Scope
IN: React/Vite SPA, rust-embed serving with SPA fallback, CRUD for routes/auth/hooks/policies/budgets, token/cost + audit analytics endpoints + views. OUT: full multi-user RBAC on the UI itself (Admin server stays private per existing model).

## Tasks
- [ ] Scaffold React + Vite SPA under `web/`, importing the existing TS SDK
- [ ] Add `rust-embed = "8"` (features axum, debug-embed); serve dist/ with SPA index fallback
- [ ] Read-model Admin endpoints: token/cost analytics from usage_events; authz_audit view
- [ ] UI: CRUD for routes / auth providers / hooks / policies / budgets
- [ ] UI: token/cost dashboard (Recharts; uPlot for dense series), code-split analytics
- [ ] Tests: backend endpoints + SPA build; typescript-review the SPA; ≥80% backend coverage
- [ ] `cargo check/clippy/test --workspace` green; SPA builds
