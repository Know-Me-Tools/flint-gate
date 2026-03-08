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
use crate::cache::GateCache;
use crate::db::Database;
use crate::proxy::{Router as GateRouter, SharedRouter};
use crate::config::SharedConfig;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the admin API.
#[derive(Clone)]
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

/// `GET /routes`
async fn list_routes_handler(State(state): State<AdminState>) -> impl IntoResponse {
    if let Some(db) = &state.db {
        match db.load_routes().await {
            Ok(routes) => Json(json!({"routes": routes})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        // Return YAML-configured routes from router
        let router = state.router.read().await;
        Json(json!({"routes": [], "note": "no database configured"})).into_response()
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
    // Inject the path id into the payload
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
        match db.load_routes().await {
            Ok(routes) => {
                if let Some(route) = routes.iter().find(|r| r.id == id) {
                    Json(json!(route)).into_response()
                } else {
                    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
                }
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
