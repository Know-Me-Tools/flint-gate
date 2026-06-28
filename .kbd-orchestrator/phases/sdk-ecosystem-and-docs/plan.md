# Plan — sdk-ecosystem-and-docs

**Phase:** sdk-ecosystem-and-docs
**Planned:** 2026-06-27
**Inputs:** `assessment.md`, `analysis.md`, `library-candidates.json`
**Change backend:** OpenSpec (`openspec/changes/`)

---

## Resolved decisions

| Question | Decision | Rationale |
|---|---|---|
| Workspace granularity | 3 crates: core (lib) + server (bin) + client (SDK) | Clean client boundary for SDK consumers |
| npm scope | `@know-me/flint-gate` | Org-scoped; conventional for npm packages |
| Docs hosting | GitHub Pages | Free, Docusaurus-native, integrates with CI |
| OpenAPI timing | This phase (utoipa annotations) | Enables SDK types + API docs in one pass |
| Stainless SDK gen | Skip | Paid; SDKs are simple enough to hand-write |

---

## Ordered change list (12 changes)

| # | Change ID | Goals | Deps | Library | Effort |
|---|---|---|---|---|---|
| 1 | `add-ci-cd` | (prereq) | none | — | Low |
| 2 | `archive-openspec-changes` | (housekeeping) | none | — | Trivial |
| 3 | `convert-to-workspace` | 3 | none | — | Medium |
| 4 | `add-openapi-spec` | 8 | #3 | `utoipa` 5.x (adopt) | Medium |
| 5 | `build-rust-client-sdk` | 3 | #3 | — | Medium |
| 6 | `build-typescript-sdk` | 4 | #4 | `tsup` (adopt) | Medium |
| 7 | `build-go-sdk` | 5 | #4 | — | Medium |
| 8 | `build-flutter-sdk` | 6 | #4 | — | Medium |
| 9 | `create-examples` | 7 | #5-8 | — | Medium |
| 10 | `create-docs-site` | 8 | #4 | Docusaurus 3 (adopt) | Medium |
| 11 | `create-claude-plugin` | 9,10 | none | — | Low |
| 12 | `create-opencode-plugin` | 11 | #11 | — | Low |

### Ordering rationale

1. **CI/CD** must land first — every subsequent change needs test gates.
2. **Archive OpenSpec** — clears the 8 done-but-unarchived changes from the prior phase.
3. **Workspace conversion** — the architectural gate. Unblocks #4 and #5.
4. **OpenAPI spec** — annotate handlers; enables SDK type generation + API docs.
5-8. **SDKs** — can be parallelized after #3/#4. TypeScript/Go/Flutter each depend on OpenAPI types. Rust client depends on workspace.
9. **Examples** — depends on all SDKs existing.
10. **Docs site** — depends on OpenAPI for API reference content.
11-12. **Plugins** — independent of SDKs; depend only on the project existing.

**Sycophancy correction** is not a separate change — it's a quality gate applied to every change's output (doc pages, SDK READMEs, skill descriptions).

---

## Per-change summaries

### Change 1: `add-ci-cd`
Create `.github/workflows/ci.yml` with: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. Add a release workflow skeleton for Trusted Publishing (crates.io OIDC). This is a prerequisite for all SDK publishing.

### Change 2: `archive-openspec-changes`
Move all 8 completed changes from `openspec/changes/` to `openspec/changes/archive/`. Sync specs to `openspec/specs/`.

### Change 3: `convert-to-workspace`
Convert single-crate project to flat workspace:
- `crates/flint-gate-core/` — lib: config, auth, stream, proxy, cache, db, admin modules
- `crates/flint-gate/` — bin: main.rs, depends on core
- `crates/flint-gate-client/` — lib: Rust client SDK stub (populated in #5)
- Root `Cargo.toml` becomes `[workspace] members = ["crates/*"]`
- Update all imports, verify `cargo test --workspace` passes

### Change 4: `add-openapi-spec`
Add `utoipa = { version = "5", features = ["axum_extras", "chrono", "uuid"] }` to `flint-gate-core`. Annotate admin API handlers with `#[utoipa::path(...)]`. Derive `ToSchema` on response types. Serve OpenAPI JSON at `/openapi.json` on the admin port. Add `utoipa-swagger-ui` for interactive docs at `/docs`.

### Change 5: `build-rust-client-sdk`
Implement `flint-gate-client` crate:
- `FlintGateClient` struct with `reqwest` backend
- `stream_sse(url, opts) -> impl Stream<SseEvent>` — SSE client with AG-UI event parsing
- `stream_ws(url, opts)` — WebSocket client via `tokio-tungstenite`
- `admin()` methods for route/key management
- Full type-safe API matching the OpenAPI spec
- Unit tests with `wiremock`

### Change 6: `build-typescript-sdk`
Create `sdks/typescript/` with:
- `package.json` (`@know-me/flint-gate`, dual ESM/CJS via `tsup`)
- `src/client.ts` — `FlintGateClient` class with `fetch` + `ReadableStream`
- `src/stream.ts` — `async function* streamSSE(): AsyncGenerator<StreamEvent>`
- `src/types.ts` — discriminated union types matching OpenAPI
- `src/ws.ts` — WebSocket client for WS protocol routes
- Framework adapters: `src/next.ts` (Next.js middleware), `src/express.ts` (Express middleware)
- `tsup.config.ts`, `tsconfig.json`, tests with `vitest`
- Publishable to npm via CI

### Change 7: `build-go-sdk`
Create `sdks/go/` with:
- `go.mod` (`github.com/know-me-tools/flint-gate/sdks/go`)
- `client.go` — `Client` struct with `net/http` backend
- `stream.go` — `StreamSSE(url, opts) (<-chan Event, error)` channel-based SSE consumer
- `types.go` — structs matching OpenAPI
- `ws.go` — WebSocket client via `gorilla/websocket` or `nhooyr.io/websocket`
- `middleware.go` — `http.Handler` middleware for Go services behind flint-gate
- Tests with `httptest`

### Change 8: `build-flutter-sdk`
Create `sdks/flutter/` with:
- `pubspec.yaml` (`flint_gate`, depends on `http` ^1.2)
- `lib/flint_gate.dart` — barrel export
- `lib/src/client.dart` — `FlintGateClient` class with `http` package
- `lib/src/sse_client.dart` — `Stream<SseEvent>` from SSE endpoint
- `lib/src/types.dart` — typed event classes matching OpenAPI
- `lib/src/auth.dart` — auth token management (interceptor pattern)
- Tests with `flutter_test`
- Achieve 140/140 pub points

### Change 9: `create-examples`
Create `examples/` with runnable projects:
- `examples/flutter-chat/` — Dart chat app consuming SSE streams
- `examples/nextjs-middleware/` — Next.js app with flint-gate middleware
- `examples/express-proxy/` — Express server behind flint-gate
- `examples/axum-middleware/` — Rust Axum service integrated with flint-gate
- `examples/tauri-desktop/` — Tauri app embedding flint-gate-client
- `examples/go-service/` — Go HTTP service behind flint-gate
- Each example has its own `README.md` + `docker-compose.yml` for one-command startup

### Change 10: `create-docs-site`
Create `docs/` with Docusaurus 3:
- `docusaurus.config.ts` — site config (GitHub Pages deployment)
- `docs/quickstart.md`, `docs/configuration.md`, `docs/deployment.md`
- `docs/sdk/rust.md`, `docs/sdk/typescript.md`, `docs/sdk/go.md`, `docs/sdk/flutter.md`
- `docs/protocols/sse.md`, `docs/protocols/websocket.md`, `docs/protocols/ndjson.md`, `docs/protocols/ag-ui.md`, `docs/protocols/a2ui.md`
- `docs/api/` — auto-generated Redoc from OpenAPI spec
- Multi-tab code blocks (`<Tabs>`) with Rust/TS/Go/Dart/cURL
- CNAME for custom domain (optional)

### Change 11: `create-claude-plugin`
Create `.claude-plugin/marketplace.json` + plugin:
- `plugin.json` manifest
- `skills/configure-gateway/SKILL.md` — config, routes, auth providers
- `skills/manage-routes/SKILL.md` — route CRUD via admin API
- `skills/manage-auth/SKILL.md` — JWT keys, API keys, auth provider setup
- `skills/deploy-gateway/SKILL.md` — Docker/K8s deployment
- `commands/gateway-health.md` — check gateway health/status
- `commands/gateway-config.md` — view/validate current config
- Validate with `skills-ref`

### Change 12: `create-opencode-plugin`
Mirror the Claude Code plugin structure for OpenCode:
- `.opencode/skills/flint-gate-config/SKILL.md`
- `.opencode/skills/flint-gate-routes/SKILL.md`
- `.opencode/skills/flint-gate-auth/SKILL.md`
- `.opencode/skills/flint-gate-deploy/SKILL.md`
- `.opencode/commands/gateway-health.md`
- `.opencode/commands/gateway-config.md`

---

## Sycophancy gate (applies to ALL changes)

After each change's artifacts (README, docs, skill descriptions) are written, run:
```
mcp__sycophancy-correction__detect_sycophancy
```
Gate: score < 0.3 to pass. Fix via `correct_sycophancy` with `mode: "rewrite"` if score >= 0.3.

---

## Execution handoff

> 12 ordered changes. Critical path: CI/CD → workspace conversion → OpenAPI → SDKs → examples/docs/plugins. Changes #5-8 (SDKs) can be parallelized after #3+#4 land. Changes #11-12 (plugins) are independent. Sycophancy correction is a quality gate on every change's output, not a separate change. Verify after each change: `cargo test --workspace && cargo clippy --workspace -- -D warnings` (Rust changes), `npm test` (TS), `go test ./...` (Go), `flutter test` (Dart).
