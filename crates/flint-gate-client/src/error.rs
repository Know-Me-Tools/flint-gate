//! Error types for the Flint Gate client SDK.

use thiserror::Error;

/// Result alias for all client operations.
pub type Result<T> = std::result::Result<T, FlintClientError>;

/// Errors returned by [`crate::FlintGateClient`] operations.
#[derive(Debug, Error)]
pub enum FlintClientError {
    /// An HTTP request failed (transport, status code outside 2xx, etc).
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// A response body could not be parsed into the expected type.
    #[error("parse error: {0}")]
    Parse(String),

    /// The server rejected the request as unauthorized or forbidden.
    #[error("auth error: {0}")]
    Auth(String),

    /// The SSE stream failed mid-flight (chunk read or framing error).
    #[error("stream error: {0}")]
    Stream(String),

    /// JSON (de)serialization failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl FlintClientError {
    /// Build a stream error from any `Display`-able value.
    pub(crate) fn stream<E: std::fmt::Display>(e: E) -> Self {
        Self::Stream(e.to_string())
    }
}
