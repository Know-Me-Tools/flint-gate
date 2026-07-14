#[allow(dead_code)]
pub mod a2ui;
#[allow(dead_code)]
pub mod ag_ui;
pub mod ndjson;
pub mod processor;
#[allow(dead_code)]
pub mod websocket;

pub use ndjson::NdjsonStreamProcessor;
pub use processor::{SseStreamProcessor, StreamMetrics};

/// The parts a stream processor needs to route human approvals: the shared
/// [`ApprovalManager`](crate::approval::ApprovalManager), this stream's decision
/// channel, and an optional config TTL override (`approval.ttl_seconds`).
pub type ApprovalHandleParts = (
    crate::approval::ApprovalManager,
    tokio::sync::mpsc::UnboundedSender<(String, crate::approval::ApprovalDecision)>,
    Option<std::time::Duration>,
);

/// Default cap (bytes) on a single assembled SSE/NDJSON event payload and on
/// the raw line buffer. Exceeding it terminates the stream (C1 DoS guard).
/// 256 KiB comfortably fits any legitimate AG-UI/A2UI event or JSON-RPC body
/// while bounding a hostile upstream that never emits a newline / blank line.
pub const DEFAULT_MAX_EVENT_BYTES: usize = 256 * 1024;

/// Default cap (bytes) on the accumulated arguments of one tool call held
/// pending authorization. Exceeding it denies that tool call (fail-closed)
/// without tearing down the stream. 1 MiB is generous for tool arguments yet
/// bounds a malicious `TOOL_CALL_ARGS` flood targeting one id.
pub const DEFAULT_MAX_TOOL_ARGS_BYTES: usize = 1024 * 1024;

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

    /// Ids of approvals currently pending in this processor. The stream task
    /// pauses upstream reads and waits for these decisions to arrive.
    #[allow(dead_code)]
    fn pending_approvals(&self) -> Vec<String> {
        Vec::new()
    }

    /// Resolve a pending approval. Returns the bytes to forward to the client,
    /// or `None` if resolving this approval produced no output.
    #[allow(dead_code)]
    fn resolve_approval(
        &mut self,
        _approval_id: &str,
        _decision: crate::approval::ApprovalDecision,
    ) -> Option<bytes::Bytes> {
        None
    }

    /// Earliest monotonic deadline among currently-pending approvals, sourced
    /// from the processor's own state (not the shared ApprovalManager).
    ///
    /// The pipeline uses this instead of `ApprovalManager::earliest_expiry` to
    /// avoid a race where the janitor purges an entry from the manager between
    /// the time the approval expires and when `sleep_until` fires — which would
    /// cause the pipeline to fall back to the 3600 s sentinel deadline.
    /// Returns `None` when no approvals are pending.
    fn earliest_pending_deadline(&self) -> Option<std::time::Instant> {
        None
    }
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

    fn pending_approvals(&self) -> Vec<String> {
        self.pending_approvals()
    }

    fn resolve_approval(
        &mut self,
        approval_id: &str,
        decision: crate::approval::ApprovalDecision,
    ) -> Option<bytes::Bytes> {
        self.resolve_approval(approval_id, decision)
    }

    fn earliest_pending_deadline(&self) -> Option<std::time::Instant> {
        self.earliest_pending_deadline()
    }
}
