# Analysis — sdk-ecosystem-and-docs

**Phase:** sdk-ecosystem-and-docs
**Analyzed:** 2026-06-27
**Mode:** Stack-specified (Rust core + multi-language SDKs)
**Research tool:** Firecrawl web search (5 searches, 25 results)
**Inputs:** `assessment.md`, web research findings

---

## Landscape summary

All ecosystem artifacts are missing. The critical path is: **workspace conversion** (unblocks everything) → **OpenAPI spec** (enables SDK generation + docs) → **SDKs** (can parallelize across languages) → **examples + docs** (can parallelize) → **skills/plugins**.

---

## Research findings (firecrawl-validated)

### Rust workspace layout

**Source:** matklad's "Large Rust Workspaces" article (rust-analyzer, 200k LOC, 32 crates)

**Verdict:** Flat `crates/*` layout with virtual manifest root. Not nested. Each directory name = crate name. Root `Cargo.toml` is `[workspace] members = ["crates/*"]` with no `[package]`.

**Pattern:**
```
Cargo.toml           # [workspace] members = ["crates/*"]
crates/
  flint-gate/        # binary (depends on core)
  flint-core/        # lib: config, auth, stream, proxy, db
  flint-client/      # lib: Rust client SDK
```

**Confidence: High.** This is the standard Rust workspace pattern used by rust-analyzer, tikv, and most large Rust projects.

### OpenAPI generation

**Source:** utoipa GitHub (3.9k stars, 6.3k dependents), Shuttle.dev guide

**ADOPT: `utoipa` 5.x + `utoipa-axum`** — Code-first OpenAPI 3.1 generation via proc macros. Has first-class axum bindings (`utoipa-axum` crate). Serves via `utoipa-swagger-ui`, `utoipa-redoc`, or `utoipa-scalar`. Used by 6,300+ Rust projects.

**Why not alternatives:**
- `paperclip` — less maintained, no axum bindings
- `openapi_type` — minimal, no ecosystem
- Hand-written YAML — maintenance burden, drift risk

### TypeScript streaming SDK patterns

**Sources:** better-sse (npm), asyncsse (GitHub), Stainless SDK generator

**Decision: BUILD** using `fetch` + `ReadableStream` (Edge-safe, no Node.js built-ins). Expose:
- `async function* stream(url, opts): AsyncGenerator<StreamEvent>` — primary API
- `EventSource` fallback for browser consumers
- Dual ESM/CJS via `tsup`
- Discriminated union: `TextDelta | ToolCall | Done | Error`

**Why not adopt `better-sse`:** It's server-side SSE (creating sessions). We need client-side consumption. The client pattern (fetch + ReadableStream + async iterator) is ~200 LOC and avoids dependencies.

### Dart/Flutter SSE packages

**Sources:** `sse` (dart-lang, 92 likes, 5.29M downloads), `flutter_client_sse` (108 likes, 52k downloads), `sse_processor` (full lifecycle)

**Decision: BUILD** — The existing packages are either bidirectional-protocol-specific (`sse` requires matching server) or too opinionated (`sse_processor` is tied to Dio + Get). A lightweight `http`-based SSE client for flint-gate's specific protocol is cleaner.

**Dependencies to use:** `http` (^1.2) for requests, Dart's native `Stream<T>` for event delivery, `flutter_lints` for static analysis.

### Claude Code plugin marketplace

**Source:** code.claude.com/docs/en/plugin-marketplaces

**Structure confirmed:**
```
.claude-plugin/
  marketplace.json    # { name, owner, plugins: [{name, source, description}] }
plugins/
  flint-gate/
    plugin.json       # { name, version, description, author, license }
    skills/
      configure-gateway/SKILL.md
      manage-routes/SKILL.md
      manage-auth/SKILL.md
    commands/
      gateway-start.md
      gateway-status.md
```

**Distribution:** Push to GitHub repo → users add via `/plugin marketplace add Know-Me-Tools/flint-gate` → `/plugin install flint-gate`.

### agentskills.io spec

**Source:** agentskills.io/specification

Required: `SKILL.md` with frontmatter (`name` ≤64 chars lowercase `[a-z0-9-]`, `description` 1-1024 chars). Body < 500 lines. References one level deep. Validate with `skills-ref validate`.

### Documentation patterns

**Sources:** Stripe Docs, Twilio Docs (Next.js+MDX rewrite), Vercel Docs

**Key patterns to adopt:**
1. Multi-tab code blocks (Rust/TS/Go/Dart/cURL) with persistent language toggle
2. Interactive "Try it" API playground
3. MDX docs-as-code with PR review
4. Algolia DocSearch or equivalent
5. Changelog as first-class citizen

**Decision: ADOPT Docusaurus 3** (OSS, React/MDX, versioning, i18n, multi-tab code blocks). Pair with Redoc for API reference (served from utoipa-generated OpenAPI spec).

---

## Build-vs-adopt decisions

| Gap | Decision | What | Why |
|---|---|---|---|
| Rust workspace | BUILD | `crates/*` flat layout | Standard pattern; modules already cleanly separated |
| OpenAPI spec | ADOPT | `utoipa` 5.x + `utoipa-axum` | Code-first, 3.9k stars, axum bindings, 6.3k users |
| Rust client SDK | BUILD | `flint-client` crate | Thin wrapper over `reqwest` + SSE parser; ~500 LOC |
| TypeScript SDK | BUILD | `fetch` + `ReadableStream` async generator | Edge-safe, dependency-free, ~300 LOC |
| Go SDK | BUILD | `net/http` + `bufio.Scanner` client | Idiomatic Go, zero deps, ~400 LOC |
| Flutter SDK | BUILD | `http` + `Stream<T>` | Matches Dart conventions, zero deps beyond `http` |
| Docs site | ADOPT | Docusaurus 3 + Redoc | OSS, customizable, multi-tab code, large ecosystem |
| CI/CD | BUILD | GitHub Actions workflows | Standard for Rust+TS+Go+Dart multi-language projects |
| Claude Code plugin | BUILD | `.claude-plugin/marketplace.json` + skills + commands | Spec confirmed via firecrawl research |
| OpenCode plugin | BUILD | Mirror `.opencode/` structure | Already has directory convention |
| Sycophancy gate | ADOPT | `mcp__sycophancy-correction__detect_sycophancy` | Already available in environment |

---

## Candidate evaluation details

### `utoipa` — OpenAPI generation

| Criterion | Finding |
|---|---|
| Stars | 3.9k |
| Dependents | 6,300+ |
| Latest version | 5.5.0 (May 2026) |
| Axum support | `utoipa-axum` crate with path parsing |
| OpenAPI version | 3.1 |
| UI serving | `utoipa-swagger-ui`, `utoipa-redoc`, `utoipa-scalar` |
| Features needed | `axum_extras`, `chrono`, `uuid` (all already deps) |
| Integration effort | Annotate handlers with `#[utoipa::path]`, derive `ToSchema` on types |

**Verdict: ADOPT.** Only viable Rust OpenAPI generator with axum bindings.

### Docusaurus 3 — Documentation site

| Criterion | Finding |
|---|---|
| Maintainer | Meta (Facebook) |
| Stars | 58k+ |
| Version | 3.x (React 18, MDX 3) |
| Multi-tab code | Built-in `<Tabs>` + `<TabItem>` components |
| Versioning | Built-in docs versioning |
| Search | Algolia DocSearch (free for OSS) |
| Deployment | GitHub Pages, Vercel, Netlify |

**Verdict: ADOPT.** Most mature OSS docs framework with multi-language code example support.

---

## Decision log entries

| ID | Decision | Rationale | Confidence |
|---|---|---|---|
| D1 | Flat `crates/*` workspace layout | Standard Rust pattern (rust-analyzer, matklad) | High |
| D2 | ADOPT `utoipa` 5.x for OpenAPI | Code-first, axum bindings, 6.3k users | High |
| D3 | ADOPT Docusaurus 3 for docs site | Most mature OSS docs framework | High |
| D4 | BUILD all four SDKs | Client-side streaming is ~300-500 LOC each; deps add bloat | High |
| D5 | BUILD GitHub Actions CI/CD | Standard for multi-language projects | High |
| D6 | BUILD Claude Code plugin with marketplace.json | Spec confirmed via firecrawl | High |
| D7 | `fetch` + `ReadableStream` for TS SDK (not `better-sse`) | Edge-safe, dependency-free | High |
| D8 | `http` + `Stream<T>` for Dart SDK (not `flutter_client_sse`) | Too opinionated; we need thin client | Medium |

---

## Open questions for /kbd-plan

1. **Workspace granularity** — Two crates (`flint-core` lib + `flint-gate` bin) or three (add `flint-client` SDK)? The two-crate split is simpler; three enables a clean client SDK boundary.
2. **TypeScript package scope** — `@know-me/flint-gate` (scoped) or `flint-gate` (unscoped) on npm? Scoped is conventional for orgs but unscoped is easier to find.
3. **Docs hosting** — GitHub Pages (free, simple) vs Vercel (fast, preview deployments)?
4. **OpenAPI auto-generation timing** — Annotate handlers as part of this phase, or defer to a follow-up? The utoipa annotations touch every handler.
5. **Stainless SDK generation** — Should we use Stainless to auto-generate TS/Go/Python SDKs from the OpenAPI spec instead of hand-writing them? ($ — paid service but eliminates manual SDK maintenance.)

No contested choices. No `/pmpo-elicit` escalation needed.
