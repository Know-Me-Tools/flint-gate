//! Bounded reads of upstream (Hydra) HTTP response bodies.
//!
//! A compromised or misbehaving upstream could return an arbitrarily large body;
//! `reqwest`'s `.json()` / `.bytes()` buffer the whole thing into memory with no
//! cap. These helpers enforce a size ceiling so a hostile upstream cannot drive
//! memory-pressure DoS, and fail **closed** (error) on over-cap rather than
//! truncating into a mis-parse.

/// Default cap for delegated Hydra responses (token exchange + introspection).
/// 64 KiB is generous for an OAuth token / introspection JSON while bounding the
/// blast radius of a hostile upstream.
pub const MAX_UPSTREAM_BODY_BYTES: usize = 64 * 1024;

/// Error reading a capped body.
#[derive(Debug, thiserror::Error)]
pub enum CappedBodyError {
    /// The body exceeded `max_bytes` (by `Content-Length` or while streaming).
    #[error("upstream response body exceeds {max} bytes")]
    TooLarge { max: usize },
    /// Transport error while streaming the body.
    #[error("error reading upstream response body: {0}")]
    Transport(String),
    /// The (capped) body was not valid JSON.
    #[error("malformed upstream JSON: {0}")]
    Json(String),
}

/// Read an upstream response body up to `max_bytes`, then parse it as JSON.
///
/// Fails closed:
/// - a `Content-Length` over the cap → [`CappedBodyError::TooLarge`] before
///   reading any body;
/// - a body that exceeds the cap while streaming → `TooLarge` (the stream is
///   dropped without buffering the overflow);
/// - a transport error → [`CappedBodyError::Transport`];
/// - invalid JSON → [`CappedBodyError::Json`].
pub async fn read_capped_json(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<serde_json::Value, CappedBodyError> {
    use futures::StreamExt;

    // Reject early on a declared Content-Length over the cap.
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(CappedBodyError::TooLarge { max: max_bytes });
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| CappedBodyError::Transport(e.to_string()))?;
        // Overflow the cap → fail closed without buffering the excess.
        // `saturating_add` so a pathological chunk length cannot wrap.
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(CappedBodyError::TooLarge { max: max_bytes });
        }
        buf.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&buf).map_err(|e| CappedBodyError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_default_is_64kib() {
        assert_eq!(MAX_UPSTREAM_BODY_BYTES, 65_536);
    }

    // The streaming path needs a live/mock server; it is exercised via the
    // token_exchange + introspect wiremock tests. Here we assert the pure
    // JSON-slice parse behavior a valid small body would hit.
    #[test]
    fn parses_valid_small_json() {
        let v: serde_json::Value =
            serde_json::from_slice(br#"{"active":true}"#).expect("valid json");
        assert_eq!(v["active"], serde_json::Value::Bool(true));
    }
}
