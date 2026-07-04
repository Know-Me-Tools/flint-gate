# add-guardrail-hook

## Summary
Add a pluggable guardrail hook interface for request/stream inspection, shipping the interface (not bundled ML models) plus a trivial reference guard. (Goal G6a)

## Design
New `guardrail/` module defining a guardrail trait (inspect → allow/block/annotate) and a `PreRequestHook::Guardrail` config variant. Ship the **interface** plus one trivial reference implementation (e.g., regex/allowlist on request body or headers). **Deliberately defer bundling prompt-injection / PII-DLP models** — those are the off-identity LLM-ops direction; the interface lets them be added later or supplied by an external service without core changes.

Library: none (trait interface + reference guard).

## Depends on
- (none — standalone; extends the PreRequestHook enum)

## Scope
IN: guardrail trait, `PreRequestHook::Guardrail` variant, one reference guard, wiring into the pipeline. OUT: bundled injection/PII models; external moderation-service integration (future).

## Tasks
- [ ] `guardrail/` module: guardrail trait (inspect → allow/block/annotate)
- [ ] `PreRequestHook::Guardrail` config variant + plumbing
- [ ] One trivial reference guard (regex/allowlist)
- [ ] Wire into `middleware/pipeline.rs`
- [ ] Tests: guard blocks/passes; ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
