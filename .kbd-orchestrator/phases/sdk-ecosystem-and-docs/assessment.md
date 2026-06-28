# Assessment — sdk-ecosystem-and-docs

**Phase:** sdk-ecosystem-and-docs
**Assessed:** 2026-06-27
**Inputs:** codebase inspection, 2026 web research, production-readiness reflection
**Goal:** Transform flint-gate from a standalone binary into a complete developer ecosystem.

---

## Baseline metrics

| Metric | Value |
|---|---|
| Rust LOC | 6,529 (25 files) |
| Tests | 74 passing |
| Project structure | Single-crate binary (`[[bin]]` only, no `[lib]`, no `[workspace]`) |
| SDKs | **0** (none exist) |
| Examples | **0** (no `examples/` directory) |
| Docs site | **None** (README.md only) |
| CI/CD | **None** (no `.github/workflows/`) |
| AI tool skills | Generic OpenSpec only (no flint-gate-specific skills) |
| Plugin manifest | **None** (no `.claude-plugin/`) |

---

## Research findings (2026 web search)

### SDK publishing standards

| Platform | Tool | Standard | Key requirement |
|---|---|---|---|
| crates.io | `cargo-release` + `cargo-semver-checks` | OIDC Trusted Publishing (no API tokens) | Workspace structure with `[workspace.package]` |
| npm | `tsup` dual `.mjs`/`.cjs` | `publint` + `are-the-types-wrong` validation | Dual ESM/CJS with per-condition `types` |
| pub.dev | `flutter_lints` + `dart format` | 140/140 pub points score | README ≤ 7,500 chars; formatted code required |
| Go modules | Git tags + `GOPROXY=proxy.golang.org` | Never mutate tagged versions | `/v2` suffix for major ≥ 2 |

### Documentation

- **Winner:** Docusaurus 3 (guides) + Redoc (API reference from OpenAPI) + multi-tab code blocks
- **Hosted alternative:** Mintlify (auto-generates MDX from OpenAPI, `/llms.txt`, MCP server)
- **World-class patterns (Stripe/Twilio/Vercel):** copy-paste code in every language per page, interactive API playground, docs-as-code with PR review, consistent response envelopes, cursor pagination, Algolia search

### Streaming SDK patterns

- **Universal contract:** async iterators + `AbortSignal` + Edge-runtime safe
- **Event types:** discriminated unions (`TextDelta | ToolCall | Done | Error`)
- **Reference implementations:** OpenAI SDK (`Stream<Chunk>`), Anthropic SDK (`MessageStream` with event emitters), Vercel AI SDK (`useChat()` hooks)

### Sycophancy correction

- 8 patterns to detect (S-01 through S-08): promotional language, inflated symbolism, vague attributions, em dash overuse, rule of three, AI vocabulary, negative parallelisms
- Detection via `mcp__sycophancy-correction__detect_sycophancy` — gate doc merges on score < 0.3
- Fix: replace marketing adjectives with measurable claims; remove triadic filler

### agentskills.io + Claude Code plugin

- **Skill spec:** `SKILL.md` with frontmatter (`name`, `description` required); body < 500 lines; references one level deep; validate with `skills-ref`
- **Claude Code plugin:** `plugin.json` manifest + `skills/` + `commands/` + `.claude-plugin/marketplace.json`; publish via GitHub-hosted marketplace repo
- **OpenCode plugin:** mirror structure under `.opencode/` with `commands/` and `skills/`

---

## Gap analysis

### Goal 1 — Web-search research update
**Status: COMPLETE (this assessment IS the research)**

All previously identified gaps from the production-readiness reflection have been validated against 2026 best practices. Key updates:
- Trusted Publishing is now the standard for crates.io (not API tokens)
- `tsup` has won as the dominant TS build tool
- Mintlify raises the bar for hosted docs (auto-OpenAPI, AI search, MCP server)
- Streaming SDKs now universally use async iterators + AbortSignal

No changes needed to the production-readiness implementations — all choices hold up.

### Goal 2 — Codebase evolution recommendations
**Status: NOT STARTED**

Requires: structured roadmap document with prioritized recommendations. Will be produced in the analyze phase.

### Goals 3-6 — SDKs (Rust, TypeScript, Go, Flutter/Dart)
**Status: ALL MISSING**

| SDK | Status | Blocking gap |
|---|---|---|
| Rust | ❌ Missing | No `[lib]` target; project is binary-only. Needs workspace conversion. |
| TypeScript | ❌ Missing | No `package.json`, no `tsconfig.json`, no source. |
| Go | ❌ Missing | No `go.mod`, no source. |
| Flutter/Dart | ❌ Missing | No `pubspec.yaml`, no source. |

**Critical path:** Rust SDK requires converting the project from a single-crate binary to a Cargo workspace with at minimum:
- `flint-gate-core` (lib: config, auth, stream, proxy logic)
- `flint-gate` (bin: the server binary, depends on core)
- `flint-gate-client` (lib: Rust client SDK)

The existing module boundaries (`auth`, `cache`, `proxy`, `stream`, `config`) map cleanly to this split.

### Goal 7 — Examples directory
**Status: MISSING**

No `examples/` directory exists. Needs runnable projects for each SDK covering:
- Flutter: SSE stream consumer chat client
- TypeScript: Next.js middleware + Express proxy + NestJS guard
- Rust: Axum middleware + Tauri desktop app
- Go: HTTP service behind flint-gate

### Goal 8 — Documentation site
**Status: MISSING**

Only `README.md` (26 KB) exists. No docs framework. Needs:
- Docusaurus 3 or Fumadocs site
- Quickstart, config reference, SDK guides per language
- API reference (auto-generated)
- Streaming protocol guides
- Deployment guides
- Multi-tab code examples

### Goals 9-11 — AI tool integration (skills, plugins)
**Status: PARTIAL (generic OpenSpec only)**

Existing `.claude/skills/` and `.opencode/skills/` contain only generic OpenSpec workflow skills (10 skills each). **Zero flint-gate-specific skills exist.** Missing:
- `SKILL.md` for flint-gate configuration
- `SKILL.md` for route/auth management
- `SKILL.md` for stream protocol setup
- `SKILL.md` for deployment
- `plugin.json` manifest
- `.claude-plugin/marketplace.json`
- OpenCode plugin structure

### Goal 12 — Sycophancy correction
**Status: NOT STARTED**

No sycophancy detection has been run on any output. Must gate all published artifacts (docs, SDK READMEs, skill descriptions) through `mcp__sycophancy-correction__detect_sycophancy`.

---

## Additional findings (not in original goals)

### CI/CD — completely missing
No `.github/workflows/` directory. The project has no CI of any kind despite:
- 74 tests that should run on every PR
- `cargo clippy -- -D warnings` that should gate merges
- Docker/K8s manifests that should be validated
- SDK publishing pipelines (once SDKs exist)

**Recommendation:** Add CI/CD as a prerequisite goal — SDKs can't publish without it.

### OpenSpec changes — 8 un-archived
All 8 production-readiness changes are implemented and done but haven't been archived via `/opsx:archive`. They should be archived and their specs synced to `openspec/specs/`.

### No OpenAPI spec
The admin API has no OpenAPI/Swagger specification. This blocks auto-generated API reference docs and SDK client generation.

---

## Recommended structure

```
flint-gate/
├── Cargo.toml                 # [workspace] root
├── crates/
│   ├── flint-gate-core/       # lib: config, auth, stream, proxy
│   ├── flint-gate/            # bin: the server (depends on core)
│   └── flint-gate-client/     # lib: Rust client SDK
├── sdks/
│   ├── typescript/            # npm package
│   ├── go/                    # Go module
│   └── flutter/               # pub.dev package
├── examples/
│   ├── flutter-chat/          # Dart SSE consumer
│   ├── nextjs-middleware/     # TS Next.js integration
│   ├── express-proxy/         # TS Express integration
│   ├── axum-middleware/       # Rust Axum integration
│   ├── tauri-desktop/         # Rust Tauri app
│   └── go-service/            # Go service behind flint-gate
├── docs/                      # Docusaurus site
│   ├── docs/
│   │   ├── quickstart.md
│   │   ├── config-reference.md
│   │   ├── sdk/
│   │   │   ├── rust.md
│   │   │   ├── typescript.md
│   │   │   ├── go.md
│   │   │   └── flutter.md
│   │   ├── protocols/
│   │   │   ├── sse.md
│   │   │   ├── websocket.md
│   │   │   ├── ndjson.md
│   │   │   ├── ag-ui.md
│   │   │   └── a2ui.md
│   │   └── deployment/
│   ├── src/
│   └── docusaurus.config.ts
├── .claude-plugin/
│   ├── marketplace.json
│   └── plugins/
│       └── flint-gate/
│           ├── plugin.json
│           ├── skills/
│           └── commands/
└── .github/
    └── workflows/
        ├── ci.yml             # test + clippy + fmt
        └── release.yml        # Trusted Publishing
```

---

## Open questions for /kbd-analyze or /kbd-plan

1. **Workspace conversion scope** — Should we split into `core` + `bin` + `client` crates, or just extract a `lib.rs` and keep a single crate? The full split is cleaner but is a significant refactor.
2. **TypeScript SDK transport** — Should the TS SDK use `fetch` + `ReadableStream` (Edge-safe) or ship two entry points (Node `http` + Edge `fetch`)?
3. **Documentation hosting** — GitHub Pages (free, simple) vs Mintlify (hosted, AI search, MCP) vs Vercel (fast, Next.js native)?
4. **OpenAPI generation** — Should we hand-write the OpenAPI spec or auto-generate from code (`utoipa` crate for Rust)?
5. **Plugin distribution** — GitHub-hosted marketplace (free, user adds repo) or submit to the official Claude marketplace?

---

## Stage gate handoff

> All ecosystem artifacts are missing — 0% of goals started. The critical path is workspace conversion (unblocks Rust SDK), then TypeScript SDK (highest user demand), then docs site + examples (can parallelize). CI/CD must be added as a prerequisite. Research phase is complete — all findings validated against 2026 best practices. 8 OpenSpec changes need archiving as housekeeping.
