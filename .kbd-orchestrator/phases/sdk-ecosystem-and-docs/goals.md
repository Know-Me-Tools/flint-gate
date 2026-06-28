# Goals — sdk-ecosystem-and-docs

Evolve flint-gate from a standalone Rust binary into a **complete developer ecosystem** with production-ready SDKs, world-class documentation, examples, and AI tool integrations.

## Objective

Build the surrounding ecosystem so that developers can integrate flint-gate into any stack — Rust, TypeScript, Go, Flutter/Dart — with first-class SDKs, copy-paste examples, comprehensive documentation, and AI agent tooling support. Research best practices via web search to ensure world-class quality.

## Goals

### Research & Recommendations
1. **Web-search research update** — Re-examine all previously identified gaps with current (2026) web research; validate that implementation choices match current best practices; identify any new industry developments that affect the codebase.
2. **Codebase evolution recommendations** — Provide a prioritized roadmap for further evolution considering documentation, skills creation, configuration ergonomics, and performance optimization.

### SDKs (production-ready, publishable)
3. **Rust SDK** (crates.io) — Client + Axum middleware + Tauri integration types. Covers: programmatic proxy config, auth provider implementation, stream processor extension, embedded gateway mode.
4. **TypeScript SDK** (npm) — Client + server middleware. Covers: Next.js middleware, Express adapter, NestJS guard, browser client for SSE/WS/NDJSON streaming.
5. **Go SDK** — Client + middleware. Covers: `net/http` middleware, gRPC gateway integration, client library for Go services.
6. **Flutter/Dart SDK** (pub.dev) — Client library. Covers: `http` interceptor, SSE/WS stream consumer, auth token management for Flutter apps.

### Examples
7. **`examples/` directory** — Runnable example projects for each SDK covering the most likely usage scenarios:
   - Flutter/Dart: chat client consuming SSE streams from flint-gate
   - TypeScript: Next.js app with flint-gate middleware + Express server proxy
   - Rust: Axum middleware integration + Tauri desktop app embedding flint-gate
   - Go: HTTP service behind flint-gate with custom auth

### Documentation
8. **World-class web documentation** — Research and implement best-in-class documentation site (Docusaurus, MkDocs Material, or equivalent). Includes: quickstart, config reference, SDK guides per language, architecture deep-dive, streaming protocol guides (SSE/WS/NDJSON/AG-UI/A2UI), deployment guides (Docker/K8s/bare metal), and API reference (auto-generated from source).

### AI Tool Integration
9. **agentskills.io compliant skill set** — Create a Claude Code / OpenCode skill package that lets AI agents configure, deploy, and troubleshoot flint-gate instances. Must pass agentskills.io spec validation.
10. **Claude Code plugin/marketplace** — Publishable `.claude-plugin` with skills, commands, and MCP server integration for flint-gate management.
11. **OpenCode plugin support** — OpenCode-compatible plugin with skills and commands mirroring the Claude Code plugin.

### Sycophancy Correction
12. **All outputs sycophancy-corrected** — Every recommendation, doc page, and skill description passes through sycophancy detection. No inflated claims, no generic AI tropes, no unfounded superlatives. Documentation describes what the system *actually does*, not what sounds impressive.

## Success Criteria

- All four SDKs publish to their respective registries (crates.io, npm, pub.dev, Go modules)
- `examples/` directory runs out-of-the-box with `docker compose up`
- Documentation site deploys and scores well on readability/clarity audits
- Skill set passes `agentskills.io` spec validation
- Claude Code plugin installs via marketplace and provides working slash commands
- Zero sycophancy-correction failures on any published artifact
