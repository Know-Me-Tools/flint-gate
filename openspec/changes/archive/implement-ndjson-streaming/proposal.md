# implement-ndjson-streaming

## Summary
Implement `NdjsonFramer` for newline-delimited JSON streaming, building on the `StreamProcessor` trait from change `extract-stream-processor-trait`.

## Motivation
`protocol: "ndjson"` is enumerated in the `StreamConfig.protocol` doc comment (`types.rs:403`) but the SSE processor's line parser (`processor.rs:143-171`) is hard-bound to SSE framing (`data:` prefix, blank-line terminator). NDJSON is one JSON object per `\n` with no prefix — a different wire format. Config accepts the value but it's silently treated as SSE.

## Design
1. **`NdjsonFramer`** implementing `StreamProcessor` (trait from change #6):
   - Split response body on `\n`.
   - Parse each non-empty line as JSON via `serde_json::from_str`.
   - Run parsed events through `EventChain` (AG-UI validation, A2UI filtering, backpressure).
   - Re-emit filtered events as `\n`-delimited JSON.

2. **Content-Type**: Ensure `Content-Type: application/x-ndjson` is set on the response when the protocol is NDJSON. Upstream may set it; if not, the processor sets it.

3. **Dispatch**: From the `match` added in change #6 at `pipeline.rs`:
   ```rust
   "ndjson" => Box::new(NdjsonFramer::new(stream_config, user_scopes, metadata, theme)),
   ```

4. **Per-protocol error shape**: On backpressure/watchdog trip, append `{"error":"session_expired"}` or `{"error":"backpressure_limit"}` as a final NDJSON line, then close the stream.

## Depends on
- `extract-stream-processor-trait` (change #6) — provides the trait and `EventChain`.

## Tasks
- [ ] Implement `NdjsonFramer` implementing `StreamProcessor`
- [ ] Add NDJSON line-splitting + JSON parse + EventChain processing
- [ ] Set Content-Type: application/x-ndjson on response
- [ ] Wire dispatch from `match stream_config.protocol` in pipeline.rs
- [ ] Implement NDJSON error-line format (`{"error":"..."}` + close)
- [ ] Add tests: line parsing, JSON parse, event filtering, backpressure error line
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
