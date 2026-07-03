//! The [`FlintGateClient`] — HTTP + SSE client for Flint Gate.

use crate::error::{FlintClientError, Result};
use crate::stream::sse_event_stream;
use crate::types::{
    ApiKey, CreateApiKeyRequest, CreatedApiKey, HealthStatus, RouteConfig, SseEvent,
};
use futures::{Stream, TryStreamExt};
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const ADMIN_PREFIX: &str = "/v1/admin";

/// Asynchronous client for Flint Gate — an AI auth proxy / API gateway.
///
/// The client is cheap to clone (it wraps an `Arc<reqwest::Client>`). Construct
/// with [`FlintGateClient::new`] for an unauthenticated client, or
/// [`FlintGateClient::with_token`] to attach a bearer token for admin routes.
///
/// ## Admin API
/// Health, readiness, route management, and API key management live under the
/// `/v1/admin` prefix on the proxy's admin listener.
///
/// ## SSE streaming
/// [`FlintGateClient::stream_sse`] issues a `POST` to the given path and
/// returns a parsed stream of [`SseEvent`]s suitable for proxying OpenAI-style
/// `text/event-stream` responses.
#[derive(Clone)]
pub struct FlintGateClient {
    base_url: Arc<str>,
    http: Client,
    token: Option<Arc<str>>,
}

impl FlintGateClient {
    /// Create a new client targeting `base_url` (e.g. `https://gate.example.com`).
    ///
    /// No authentication token is attached. Admin endpoints requiring auth
    /// will return 401.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Self::with_options(base_url, None)
    }

    /// Create a new client with a bearer `token` used for all requests.
    ///
    /// The token is sent as `Authorization: Bearer <token>` on every request,
    /// including both admin API calls and proxied streaming calls.
    pub fn with_token(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        Self::with_options(base_url, Some(token.into()))
    }

    fn with_options(base_url: impl Into<String>, token: Option<String>) -> Result<Self> {
        let raw = base_url.into();
        let trimmed = raw.trim_end_matches('/');
        let http = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            // Streaming requests opt out of the overall timeout via
            // `stream_sse`, which uses a dedicated request builder.
            .build()
            .map_err(FlintClientError::from)?;

        Ok(Self {
            base_url: trimmed.into(),
            http,
            token: token.map(|t| t.into()),
        })
    }

    /// Returns the base URL the client was constructed with.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured bearer token, if any.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Build a fully-qualified admin URL for `path` (which must start with `/`).
    fn admin_url(&self, path: &str) -> String {
        // path is a fragment like "/routes"; we prepend the admin prefix.
        // Callers may also pass a path that already includes the prefix —
        // support that for ergonomics.
        if path.starts_with(ADMIN_PREFIX) {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}{}", self.base_url, ADMIN_PREFIX, path)
        }
    }

    fn add_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(t) = &self.token {
            req = req.bearer_auth(t.as_ref());
        }
        req
    }

    async fn send_admin<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T> {
        let url = self.admin_url(path);
        let mut req = self.http.request(method, &url);
        req = self.add_auth(req);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(FlintClientError::from)?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let txt = resp.text().await.unwrap_or_default();
            return Err(FlintClientError::Auth(format!("{}: {}", status, txt)));
        }
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(FlintClientError::Auth(format!("{}: {}", status, txt)));
        }
        let parsed: T = resp.json().await.map_err(FlintClientError::from)?;
        Ok(parsed)
    }

    // ── Health / readiness ───────────────────────────────────────────────────

    /// `GET /v1/admin/health` — liveness probe.
    pub async fn health(&self) -> Result<HealthStatus> {
        self.send_admin(Method::GET, "/health", None).await
    }

    /// `GET /v1/admin/ready` — readiness probe (checks DB connectivity).
    pub async fn ready(&self) -> Result<HealthStatus> {
        self.send_admin(Method::GET, "/ready", None).await
    }

    // ── Route management ─────────────────────────────────────────────────────

    /// `GET /v1/admin/routes` — list all configured routes.
    ///
    /// Returns the raw route documents as stored by the server (the source
    /// field — `"database"` or `"config"` — is preserved in each element's
    /// `extra` map).
    pub async fn list_routes(&self) -> Result<Vec<RouteConfig>> {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ListRoutesEnvelope {
            routes: Vec<serde_json::Value>,
            #[serde(default)]
            source: Option<String>,
        }
        let envelope: ListRoutesEnvelope = self.send_admin(Method::GET, "/routes", None).await?;
        Ok(envelope
            .routes
            .into_iter()
            .map(|v| {
                serde_json::from_value(v).unwrap_or(RouteConfig {
                    id: String::new(),
                    priority: 0,
                    extra: serde_json::Value::Null,
                })
            })
            .collect())
    }

    /// `POST /v1/admin/routes` — create or update (upsert) a route.
    pub async fn create_route(&self, route: &RouteConfig) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct UpsertResp {
            id: String,
        }
        // Serialize the typed fields back into the raw document.
        let mut body = match &route.extra {
            serde_json::Value::Object(map) => serde_json::Value::Object(map.clone()),
            _ => serde_json::json!({}),
        };
        if let serde_json::Value::Object(ref mut map) = body {
            map.insert("id".to_string(), serde_json::json!(route.id));
            map.insert("priority".to_string(), serde_json::json!(route.priority));
        }
        let resp: UpsertResp = self
            .send_admin(Method::POST, "/routes", Some(&body))
            .await?;
        Ok(resp.id)
    }

    /// `DELETE /v1/admin/routes/{id}` — delete a route. Returns `true` if a
    /// route was actually removed.
    pub async fn delete_route(&self, id: &str) -> Result<bool> {
        #[derive(serde::Deserialize)]
        struct DeleteResp {
            status: String,
        }
        let path = format!("/routes/{}", urlencoding_encode(id));
        let resp: DeleteResp = self.send_admin(Method::DELETE, &path, None).await?;
        Ok(resp.status.eq_ignore_ascii_case("deleted"))
    }

    // ── API key management ───────────────────────────────────────────────────

    /// `GET /v1/admin/api-keys` — list active API keys (metadata only).
    pub async fn list_api_keys(&self) -> Result<Vec<ApiKey>> {
        #[derive(serde::Deserialize)]
        struct ListEnvelope {
            #[serde(default, rename = "api_keys")]
            api_keys: Vec<ApiKey>,
        }
        let envelope: ListEnvelope = self.send_admin(Method::GET, "/api-keys", None).await?;
        Ok(envelope.api_keys)
    }

    /// `POST /v1/admin/api-keys` — create a new API key.
    ///
    /// The raw key is returned exactly once in [`CreatedApiKey::key`]. The
    /// caller is responsible for persisting it securely.
    pub async fn create_api_key(&self, req: &CreateApiKeyRequest) -> Result<CreatedApiKey> {
        self.send_admin(Method::POST, "/api-keys", Some(&serde_json::to_value(req)?))
            .await
    }

    // ── SSE streaming ────────────────────────────────────────────────────────

    /// Issue a `POST` to `path` (relative to `base_url`, e.g. `/v1/chat/completions`)
    /// with the given JSON body and return a parsed stream of SSE events.
    ///
    /// The request carries the client's bearer token if one is configured.
    /// No overall read timeout is applied to the response; the caller may drop
    /// the returned stream to abort.
    pub async fn stream_sse<B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>>> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.post(&url).json(body);
        req = self.add_auth(req);
        let resp = req.send().await.map_err(FlintClientError::from)?;
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let txt = resp.text().await.unwrap_or_default();
            return Err(FlintClientError::Auth(format!("{}: {}", status, txt)));
        }
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(FlintClientError::Auth(format!("{}: {}", status, txt)));
        }
        Ok(sse_event_stream(resp))
    }

    /// Convenience wrapper around [`Self::stream_sse`] that drives the stream
    /// to completion, returning every event in a `Vec`. Useful for tests and
    /// short-lived streams.
    pub async fn collect_sse<B: Serialize>(&self, path: &str, body: &B) -> Result<Vec<SseEvent>> {
        let stream = self.stream_sse(path, body).await?;
        stream.try_collect().await
    }
}

/// Minimal path-segment percent-encoder for route IDs.
///
/// We avoid pulling in an extra dep by handling only the characters the SSE
/// parser cares about; route IDs are typically opaque strings or UUIDs.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
