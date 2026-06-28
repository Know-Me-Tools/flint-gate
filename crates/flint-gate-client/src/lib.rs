//! Flint Gate client SDK — async HTTP + SSE client for the Flint Gate
//! AI auth proxy / API gateway.
//!
//! ## Quick start
//!
//! ```no_run
//! # use flint_gate_client::{FlintGateClient, types::CreateApiKeyRequest};
//! # async fn run() -> flint_gate_client::Result<()> {
//! let client = FlintGateClient::with_token(
//!     "https://gate.example.com",
//!     "admin-token",
//! )?;
//!
//! // Liveness / readiness
//! let health = client.health().await?;
//! println!("health: {:?}", health);
//!
//! // Manage routes
//! let routes = client.list_routes().await?;
//!
//! // Create an API key (raw key returned once!)
//! let created = client
//!     .create_api_key(&CreateApiKeyRequest {
//!         client_id: "billing-svc".into(),
//!         scopes: vec!["chat".into()],
//!         expires_at: None,
//!     })
//!     .await?;
//! println!("store this: {}", created.key);
//!
//! // Stream an SSE endpoint (e.g. OpenAI chat completions)
//! use serde_json::json;
//! let mut stream = client
//!     .stream_sse("/v1/chat/completions", &json!({"stream": true}))
//!     .await?;
//! while let Some(event) = futures::StreamExt::next(&mut stream).await {
//!     match event {
//!         Ok(ev) if ev.is_done() => break,
//!         Ok(ev) => println!("data: {}", ev.data),
//!         Err(e) => return Err(e),
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Module layout
//!
//! - [`client`] — the [`FlintGateClient`] struct and admin API methods
//! - [`stream`] — SSE framing parser (also exported via `stream_sse`)
//! - [`types`] — request/response DTOs
//! - [`error`] — `FlintClientError` and `Result` alias

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod error;
pub mod stream;
pub mod types;

pub use client::FlintGateClient;
pub use error::{FlintClientError, Result};
pub use types::{
    ApiKey, CreateApiKeyRequest, CreatedApiKey, HealthStatus, RouteConfig, SseEvent,
};

/// Crate version, mirroring `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
