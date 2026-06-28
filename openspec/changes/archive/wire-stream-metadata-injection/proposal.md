# wire-stream-metadata-injection

## Summary
Wire AG-UI `inject_metadata` template resolution and A2UI `theme` injection into the streaming pipeline. Both are half-wired at the same fix point.

## Motivation
**Goal 6 — AG-UI metadata:** `AgUiConfig.inject_metadata: HashMap<String,String>` (`types.rs:436-438`) is parsed but the pipeline always passes an empty map. At `processor.rs:201`, `let meta = serde_json::Map::new();` is hard-coded. The `AgUiProcessor::process` mechanism (`ag_ui.rs:143-147, 66-70`) is complete — only the pipeline never populates the map.

**Goal 7 — A2UI theme:** `A2UiProcessor::process` accepts `theme: Option<Value>` (`a2ui.rs:85`) and calls `inject_theme` (`a2ui.rs:107-109`). The call site at `processor.rs:219` hard-codes `None`. `A2UiConfig` (`types.rs:442-448`) has no `theme` field at all.

## Design

### AG-UI metadata injection (Goal 6)
1. In `pipeline.rs` around line 427 (before `SseStreamProcessor::new`), render each entry of `matched_route.config.stream.ai.ag_ui.inject_metadata` via `TemplateEngine::render` against the in-scope `template_ctx` (built at `pipeline.rs:225`).
2. Convert resulting `HashMap<String, String>` to `serde_json::Map<String, Value>` — parse each value as JSON first, fall back to string (same pattern as `apply_body_transforms` at `pipeline.rs:518-525`).
3. Pass the map into `SseStreamProcessor::new` as a new parameter.
4. Store on `SseStreamProcessor` and replace `serde_json::Map::new()` at `processor.rs:201` with the stored map (clone per event, since `inject_metadata` consumes it).

### A2UI theme injection (Goal 7)
1. Add `theme: Option<serde_json::Value>` to `A2UiConfig` in `types.rs:442-448` (with `#[serde(default)]`).
2. Thread it through `SseStreamProcessor::new` alongside the AG-UI metadata change.
3. Replace literal `None` at `processor.rs:219` with `self.theme.clone()`.

## Tasks
- [ ] Add `theme: Option<serde_json::Value>` to `A2UiConfig` with `#[serde(default)]`
- [ ] Extend `SseStreamProcessor::new` signature to accept `metadata: Map<String,Value>` and `theme: Option<Value>`
- [ ] In `pipeline.rs`, render `inject_metadata` templates against `template_ctx` before processor construction
- [ ] Convert rendered templates to `serde_json::Map` (parse-as-JSON-else-string)
- [ ] Replace `serde_json::Map::new()` at `processor.rs:201` with stored metadata
- [ ] Replace `None` at `processor.rs:219` with `self.theme.clone()`
- [ ] Update `config.example.yaml` with `theme` example under `a2ui`
- [ ] Add tests: metadata template resolution, theme passthrough, empty-config no-op
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
