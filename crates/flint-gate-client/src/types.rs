//! Public type definitions for the Flint Gate client SDK.

use serde::{Deserialize, Serialize};

/// A single Server-Sent Event parsed from the stream.
///
/// Per the SSE spec, `data` may be composed of multiple `data:` lines joined
/// with `\n`. `event` defaults to `"message"` when not specified by the server.
/// `id` is optional and carries the Last-Event-ID value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field value (defaults to `"message"`).
    pub event: String,
    /// The joined `data:` payload. For the OpenAI `[DONE]` sentinel, this is
    /// the literal `"[DONE]"` string and `event` is `"done"`.
    pub data: String,
    /// The optional `id:` field value.
    pub id: Option<String>,
}

impl SseEvent {
    /// Attempt to deserialize the `data` payload as JSON into `T`.
    pub fn data_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.data)
    }

    /// Returns `true` if this event is the `[DONE]` sentinel.
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]" || self.event == "done"
    }
}

/// Liveness / readiness probe response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// `"ok"` for health, `"ready"`/`"not ready"` for readiness.
    pub status: String,
    /// `"flint-gate"` for health responses.
    #[serde(default)]
    pub service: Option<String>,
    /// Only populated by `/ready` — `"ok"`, `"not configured"`, or an error.
    #[serde(default)]
    pub db: Option<String>,
}

/// A route definition returned by `GET /routes` or `POST /routes`.
///
/// The full route body is opaque JSON (the server forwards it to the DB), so
/// we keep the typed fields the SDK cares about and stash the rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteConfig {
    /// Unique route identifier. Required for upsert.
    pub id: String,
    /// Lower numbers match later; default `0`.
    #[serde(default)]
    pub priority: i32,
    /// The raw route document as stored by the server.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Metadata for an API key as returned by `GET /api-keys`.
///
/// Raw key material is never exposed by the list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Server-assigned key identifier (UUID).
    pub id: String,
    /// Client identifier this key authenticates.
    pub client_id: String,
    /// Granted scopes (e.g. `["chat"]`).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// RFC-3339 expiry, if any.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Response body for `POST /api-keys` — includes the raw key (shown once).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedApiKey {
    /// Server-assigned key identifier (UUID).
    pub id: String,
    /// Client identifier this key authenticates.
    pub client_id: String,
    /// Granted scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// RFC-3339 expiry, if any.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// The raw key. Store this securely — the server cannot recover it.
    pub key: String,
}

/// Request body for `POST /api-keys`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Client identifier this key will authenticate.
    pub client_id: String,
    /// Scopes to grant (e.g. `["chat"]`).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// RFC-3339 expiry, if any.
    #[serde(default)]
    pub expires_at: Option<String>,
}
