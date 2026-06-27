/// Admin API — internal-only Axum router on port 4457.
///
/// Endpoints:
/// - `GET  /health` — liveness probe
/// - `GET  /ready`  — readiness probe (checks DB)
/// - `GET  /cache/stats` — cache entry counts
/// - `POST /cache/invalidate` — manual cache flush
/// - `GET  /routes` — list all routes
/// - `POST /routes` — create/update a route
/// - `GET  /routes/:id` — get a route
/// - `PUT  /routes/:id` — update a route
/// - `DELETE /routes/:id` — delete a route
/// - `GET  /api-keys` — list active API keys (metadata only)
/// - `POST /api-keys` — create a new API key (returns raw key once)
/// - `DELETE /api-keys/:id` — revoke an API key
use crate::cache::GateCache;
use crate::config::SharedConfig;
use crate::db::Database;
use crate::proxy::SharedRouter;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Shared state for the admin API.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AdminState {
    pub cache: Arc<GateCache>,
    pub db: Option<Arc<Database>>,
    pub router: SharedRouter,
    pub config: SharedConfig,
}

/// Build the admin Axum router.
pub fn admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/cache/stats", get(cache_stats_handler))
        .route("/cache/invalidate", post(cache_invalidate_handler))
        .route("/routes", get(list_routes_handler).post(upsert_route_handler))
        .route(
            "/routes/:id",
            get(get_route_handler)
                .put(upsert_route_handler_with_id)
                .delete(delete_route_handler),
        )
        .route("/api-keys", get(list_api_keys_handler).post(create_api_key_handler))
        .route("/api-keys/:id", axum::routing::delete(revoke_api_key_handler))
        .route("/signing-keys", get(list_signing_keys_handler).post(create_signing_key_handler))
        .route("/signing-keys/:id", axum::routing::delete(deactivate_signing_key_handler))
        .with_state(state)
}

/// `GET /health` — always 200.
async fn health_handler() -> impl IntoResponse {
    Json(json!({"status": "ok", "service": "flint-gate"}))
}

/// `GET /ready` — checks DB connectivity if configured.
async fn ready_handler(State(state): State<AdminState>) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match sqlx::query("SELECT 1").fetch_one(&db.pool()).await {
            Ok(_) => Json(json!({"status": "ready", "db": "ok"})).into_response(),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status": "not ready", "db": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        Json(json!({"status": "ready", "db": "not configured"})).into_response()
    }
}

/// `GET /cache/stats`
async fn cache_stats_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let stats = state.cache.stats();
    Json(json!(stats))
}

/// `POST /cache/invalidate`
async fn cache_invalidate_handler(State(state): State<AdminState>) -> impl IntoResponse {
    state.cache.invalidate_all().await;
    Json(json!({"status": "invalidated"}))
}

/// `GET /routes` — returns DB routes when available, else YAML-configured routes.
async fn list_routes_handler(State(state): State<AdminState>) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.load_routes().await {
            Ok(routes) => Json(json!({"routes": routes, "source": "database"})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        let router = state.router.read().await;
        let route_ids: Vec<String> = router.route_ids().collect();
        Json(json!({"routes": route_ids, "source": "config", "note": "no database configured"}))
            .into_response()
    }
}

/// `POST /routes` — create or update a route (upsert).
async fn upsert_route_handler(
    State(state): State<AdminState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    upsert_route_inner(state, payload).await
}

/// `PUT /routes/:id` — update route with explicit ID.
async fn upsert_route_handler_with_id(
    Path(id): Path<String>,
    State(state): State<AdminState>,
    Json(mut payload): Json<Value>,
) -> impl IntoResponse {
    if let Value::Object(ref mut map) = payload {
        map.insert("id".to_string(), json!(id));
    }
    upsert_route_inner(state, payload).await
}

async fn upsert_route_inner(state: AdminState, payload: Value) -> axum::response::Response {
    let id = match payload.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing route id"})),
            )
                .into_response();
        }
    };

    let priority = payload
        .get("priority")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    if let Some(db) = &state.db {
        match db.upsert_route(&id, &payload, priority).await {
            Ok(_) => Json(json!({"status": "ok", "id": id})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `GET /routes/:id`
async fn get_route_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.get_route(&id).await {
            Ok(Some(route)) => Json(json!(route)).into_response(),
            Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `DELETE /routes/:id`
async fn delete_route_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.delete_route(&id).await {
            Ok(true) => Json(json!({"status": "deleted", "id": id})).into_response(),
            Ok(false) => {
                (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

// ── API key management ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateApiKeyRequest {
    client_id: String,
    #[serde(default)]
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
}

/// `GET /api-keys` — list active API keys (no key hashes returned).
async fn list_api_keys_handler(State(state): State<AdminState>) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.list_api_keys().await {
            Ok(keys) => {
                let items: Vec<Value> = keys
                    .into_iter()
                    .map(|k| json!({
                        "id": k.id,
                        "client_id": k.client_id,
                        "scopes": k.scopes,
                        "expires_at": k.expires_at,
                    }))
                    .collect();
                Json(json!({"api_keys": items})).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `POST /api-keys` — create a new API key.
///
/// Returns the raw key in the response body. This is the ONLY time the raw key
/// is accessible; it cannot be recovered later.
async fn create_api_key_handler(
    State(state): State<AdminState>,
    Json(payload): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.create_api_key(&payload.client_id, &payload.scopes, payload.expires_at).await {
            Ok((id, raw_key)) => (
                StatusCode::CREATED,
                Json(json!({
                    "id": id,
                    "client_id": payload.client_id,
                    "scopes": payload.scopes,
                    "expires_at": payload.expires_at,
                    "key": raw_key,
                    "note": "Store this key securely — it will not be shown again.",
                })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `DELETE /api-keys/:id` — revoke (soft-delete) an API key.
async fn revoke_api_key_handler(
    Path(id): Path<Uuid>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.revoke_api_key(id).await {
            Ok(true) => Json(json!({"status": "revoked", "id": id})).into_response(),
            Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

// ── JWT signing key management ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateSigningKeyRequest {
    id: String,
    algorithm: String,
    public_key: String,
    private_key: String,
}

/// `GET /signing-keys` — list all signing keys (public keys only).
async fn list_signing_keys_handler(State(state): State<AdminState>) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.list_signing_keys().await {
            Ok(keys) => Json(json!({"signing_keys": keys})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `POST /signing-keys` — insert a new signing key (deactivates all others).
async fn create_signing_key_handler(
    State(state): State<AdminState>,
    Json(payload): Json<CreateSigningKeyRequest>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db
            .insert_signing_key(
                &payload.id,
                &payload.algorithm,
                &payload.public_key,
                &payload.private_key,
            )
            .await
        {
            Ok(_) => (
                StatusCode::CREATED,
                Json(json!({
                    "status": "activated",
                    "id": payload.id,
                    "algorithm": payload.algorithm,
                    "note": "All prior signing keys deactivated."
                })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}

/// `DELETE /signing-keys/:id` — deactivate a signing key.
async fn deactivate_signing_key_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.deactivate_signing_key(&id).await {
            Ok(true) => Json(json!({"status": "deactivated", "id": id})).into_response(),
            Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "database not configured"})),
        )
            .into_response()
    }
}
