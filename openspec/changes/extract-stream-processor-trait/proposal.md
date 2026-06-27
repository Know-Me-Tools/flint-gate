# extract-stream-processor-trait

## Summary
Extract a `StreamProcessor` trait from `SseStreamProcessor`, refactor SSE into `SseFramer`, implement `WsFramer` for WebSocket streaming using `tokio-tungstenite`, and add protocol dispatch in the pipeline.

## Motivation
`SseStreamProcessor` (`src/stream/processor.rs:55-250`) is the only stream processor and is hard-bound to the SSE wire format (`data:`/`event:`/blank-line). The config accepts `protocol: "websocket"` and `protocol: "ndjson"` (`types.rs:403-405`) but `pipeline.rs:427` unconditionally constructs `SseStreamProcessor`. WebSocket and NDJSON are silently treated as SSE.

**Library: adopt `tokio-tungstenite` 0.28** (`library-candidates.json` D1). Already in lockfile at 0.28.0 via axum ws feature.

## Design

### Step 1: Trait extraction
```rust
pub trait StreamProcessor: Send {
    fn process_chunk(&mut self, bytes: Bytes) -> Option<Vec<u8>>;
    fn flush_event(&mut self) -> Option<Vec<u8>>;
    fn metrics(&self) -> &StreamMetrics;
    fn terminated(&self) -> bool;
}
```

### Step 2: Refactor SseStreamProcessor → SseFramer
- Move SSE wire-format logic (`process_line` at `processor.rs:143-171`) into `SseFramer`.
- Extract AG-UI, A2UI, backpressure into a shared `EventChain` struct that any framer composes.
- `SseFramer` implements `StreamProcessor`.

### Step 3: Implement WsFramer
- Add `tokio-tungstenite = { version = "0.28", features = ["rustls-tls-webpki-roots"] }` as direct dep in `Cargo.toml` (promote from transitive).
- `WsFramer` calls `tokio_tungstenite::connect_async(upstream_url)` to establish upstream WS connection.
- **Terminate-and-replay mode**: inspect `Message::Text` frames for AG-UI JSON; run through `EventChain` (same validation/metering/filtering as SSE).
- Bidirectional pipe: forward client → upstream and upstream → client frames, applying the event chain on upstream → client text frames.
- Per-protocol error: WS close frame with code 1008 (policy violation) on backpressure/watchdog trip.

### Step 4: Protocol dispatch in pipeline.rs
Insert between `pipeline.rs:417` and `427`:
```rust
let processor: Box<dyn StreamProcessor> = match stream_config.protocol.as_str() {
    "websocket" => Box::new(WsFramer::new(...)),
    "ndjson" => Box::new(NdjsonFramer::new(...)),  // change #7
    _ => Box::new(SseFramer::new(...)),
};
```

### Step 5: WebSocket upgrade detection
In the proxy handler (`pipeline.rs:64`), detect `Upgrade: websocket` headers before the SSE path. If the route's stream protocol is `websocket` and the request is a WS upgrade, branch to the WS handler. Otherwise fall through to HTTP/SSE.

## Tasks
- [ ] Define `StreamProcessor` trait in `src/stream/mod.rs`
- [ ] Extract `EventChain` (AG-UI + A2UI + backpressure) from SseStreamProcessor
- [ ] Refactor SseStreamProcessor → SseFramer implementing StreamProcessor
- [ ] Add `tokio-tungstenite = { version = "0.28", features = ["rustls-tls-webpki-roots"] }` as direct dep
- [ ] Implement `WsFramer` with upstream `connect_async` + terminate-and-replay event chain
- [ ] Add protocol dispatch (`match stream_config.protocol`) in `pipeline.rs`
- [ ] Add WS upgrade detection in proxy handler
- [ ] Ensure existing SSE tests pass after refactor (adapt as needed)
- [ ] Add WsFramer unit tests (frame parsing, AG-UI on WS, backpressure close 1008)
- [ ] `cargo test --workspace && cargo clippy -- -D warnings`
