#[allow(dead_code)]
pub mod ag_ui;
#[allow(dead_code)]
pub mod a2ui;
pub mod ndjson;
pub mod processor;
#[allow(dead_code)]
pub mod websocket;

pub use ndjson::NdjsonStreamProcessor;
pub use processor::{SseStreamProcessor, StreamMetrics};

/// Trait for all stream protocol processors (SSE, NDJSON, WebSocket).
///
/// Each processor wraps the wire-format framing while sharing the same
/// AG-UI/A2UI/backpressure event chain.
pub trait StreamProcessor: Send {
    /// Process a raw chunk of bytes from the upstream stream.
    ///
    /// Returns the filtered/processed bytes to forward to the client.
    /// Returns `None` if a backpressure limit or watchdog has terminated the stream.
    fn process_chunk(&mut self, chunk: &[u8]) -> Option<bytes::Bytes>;

    /// Return a snapshot of the current stream metrics.
    fn metrics(&self) -> &StreamMetrics;

    /// Whether the stream was terminated by a backpressure limit.
    #[allow(dead_code)]
    fn terminated_by_limit(&self) -> bool;

    /// Protocol-specific error message emitted on termination.
    fn termination_payload(&self) -> Vec<u8>;
}

impl StreamProcessor for SseStreamProcessor {
    fn process_chunk(&mut self, chunk: &[u8]) -> Option<bytes::Bytes> {
        self.process_chunk(chunk)
    }

    fn metrics(&self) -> &StreamMetrics {
        self.metrics()
    }

    fn terminated_by_limit(&self) -> bool {
        self.metrics().terminated_by_limit
    }

    fn termination_payload(&self) -> Vec<u8> {
        b"data: {\"type\":\"RUN_ERROR\",\"message\":\"stream limit exceeded\"}\n\n".to_vec()
    }
}
