# Rust SDK

The Rust SDK is the `flint-gate-client` crate. It provides an async HTTP client, SSE stream consumer, WebSocket client, and admin API bindings.

## Install

Add to `Cargo.toml`:

```toml
[dependencies]
flint-gate-client = { git = "https://github.com/know-me-tools/flint-gate" }
```

Or use a path dependency during development:

```toml
flint-gate-client = { path = "../crates/flint-gate-client" }
```

## Quickstart

```rust
use flint_gate_client::{FlintGateClient, types::CreateApiKeyRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> flint_gate_client::Result<()> {
    let client = FlintGateClient::with_token(
        "https://gate.example.com",
        "admin-token",
    )?;

    // Health / readiness
    let health = client.health().await?;
    println!("{:?}", health);

    // List routes
    let routes = client.list_routes().await?;

    // Create an API key — the raw key is returned only once
    let created = client
        .create_api_key(&CreateApiKeyRequest {
            client_id: "billing-svc".into(),
            scopes: vec!["chat".into()],
            expires_at: None,
        })
        .await?;
    println!("store this: {}", created.key);

    // Stream an SSE endpoint
    let mut stream = client
        .stream_sse("/v1/chat/completions", &json!({"stream": true}))
        .await?;

    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        match event {
            Ok(ev) if ev.is_done() => break,
            Ok(ev) => println!("data: {}", ev.data),
            Err(e) => return Err(e),
        }
    }

    Ok(())
}
```

## Admin methods

| Method | Endpoint |
|--------|----------|
| `health()` | `GET /health` |
| `ready()` | `GET /ready` |
| `list_routes()` | `GET /routes` |
| `get_route(id)` | `GET /routes/{id}` |
| `upsert_route(...)` | `POST /routes` |
| `delete_route(id)` | `DELETE /routes/{id}` |
| `list_api_keys()` | `GET /api-keys` |
| `create_api_key(...)` | `POST /api-keys` |
| `revoke_api_key(id)` | `DELETE /api-keys/{id}` |

## Error handling

All async methods return `flint_gate_client::Result<T>`, aliased to `std::result::Result<T, FlintClientError>`. Inspect `FlintClientError` for HTTP status, parse failures, or transport errors.

## Testing

```bash
cargo test -p flint-gate-client
```
