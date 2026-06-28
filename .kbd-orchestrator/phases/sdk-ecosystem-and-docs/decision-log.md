# Decision Log — sdk-ecosystem-and-docs

| ID | Date | Decision | Rationale | Confidence |
|---|---|---|---|---|
| D1 | 2026-06-27 | BUILD flat `crates/*` workspace layout | Standard Rust pattern (matklad, rust-analyzer) | High |
| D2 | 2026-06-27 | ADOPT `utoipa` 5.x for OpenAPI generation | Code-first, axum bindings, 3.9k stars, 6.3k dependents | High |
| D3 | 2026-06-27 | ADOPT Docusaurus 3 for documentation site | Most mature OSS docs framework; multi-tab code, versioning, i18n | High |
| D4 | 2026-06-27 | BUILD all four SDKs (Rust, TS, Go, Dart) | Client streaming is 300-500 LOC each; external deps add bloat | High |
| D5 | 2026-06-27 | BUILD GitHub Actions CI/CD pipelines | Standard for multi-language projects; OIDC Trusted Publishing | High |
| D6 | 2026-06-27 | BUILD Claude Code plugin with marketplace.json | Spec confirmed via firecrawl research (code.claude.com) | High |
| D7 | 2026-06-27 | TS SDK uses `fetch` + `ReadableStream` (not `better-sse`) | Edge-safe, dependency-free; better-sse is server-side only | High |
| D8 | 2026-06-27 | Dart SDK uses `http` + `Stream<T>` (not `flutter_client_sse`) | Existing packages too opinionated or protocol-specific | Medium |
