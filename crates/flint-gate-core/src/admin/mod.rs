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
use crate::authz::{policy_warnings, validate_policy, AuthzEngine, PolicyRecord};
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
#[allow(unused_imports)]
use utoipa::ToSchema;
use uuid::Uuid;

/// Shared state for the admin API.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AdminState {
    pub cache: Arc<GateCache>,
    pub db: Option<Arc<Database>>,
    pub router: SharedRouter,
    pub config: SharedConfig,
    /// Shared Cedar authorization engine. Policy writes validate then reload it.
    pub authz: Arc<AuthzEngine>,
}

/// Build the admin Axum router.
pub fn admin_router(state: AdminState) -> Router {
    use utoipa::OpenApi;
    use utoipa_swagger_ui::SwaggerUi;

    #[derive(OpenApi)]
    #[openapi(
        info(
            title = "Flint Gate Admin API",
            version = "0.1.0",
            description = "Admin API for Flint Gate — AI-native auth proxy and API gateway",
            license(name = "MIT"),
        ),
        paths(health_handler, ready_handler)
    )]
    struct AdminApiDoc;

    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/cache/stats", get(cache_stats_handler))
        .route("/cache/invalidate", post(cache_invalidate_handler))
        .route(
            "/routes",
            get(list_routes_handler).post(upsert_route_handler),
        )
        .route(
            "/routes/:id",
            get(get_route_handler)
                .put(upsert_route_handler_with_id)
                .delete(delete_route_handler),
        )
        .route(
            "/api-keys",
            get(list_api_keys_handler).post(create_api_key_handler),
        )
        .route(
            "/api-keys/:id",
            axum::routing::delete(revoke_api_key_handler),
        )
        .route(
            "/signing-keys",
            get(list_signing_keys_handler).post(create_signing_key_handler),
        )
        .route(
            "/signing-keys/:id",
            axum::routing::delete(deactivate_signing_key_handler),
        )
        .route(
            "/policies",
            get(list_policies_handler).post(create_policy_handler),
        )
        .route(
            "/policies/:id",
            get(get_policy_handler)
                .put(update_policy_handler)
                .delete(delete_policy_handler),
        )
        .merge(SwaggerUi::new("/docs").url("/openapi.json", AdminApiDoc::openapi()))
        .with_state(state)
}

/// `GET /health` — always 200.
#[utoipa::path(get, path = "/health", responses((status = 200, description = "Service healthy")))]
async fn health_handler() -> impl IntoResponse {
    Json(json!({"status": "ok", "service": "flint-gate"}))
}

/// `GET /ready` — checks DB connectivity if configured.
#[utoipa::path(get, path = "/ready", responses((status = 200, description = "Ready"), (status = 503, description = "Not ready — DB unreachable")))]
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
            Ok(None) => {
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
                    .map(|k| {
                        json!({
                            "id": k.id,
                            "client_id": k.client_id,
                            "scopes": k.scopes,
                            "expires_at": k.expires_at,
                        })
                    })
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
        match db
            .create_api_key(&payload.client_id, &payload.scopes, payload.expires_at)
            .await
        {
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

// ── Authorization policy management ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UpsertPolicyRequest {
    /// Required on POST; supplied via the path on PUT.
    #[serde(default)]
    id: Option<String>,
    policy_text: String,
    #[serde(default)]
    schema_json: Option<Value>,
    #[serde(default)]
    entities_json: Option<Value>,
    #[serde(default = "default_policy_enabled")]
    enabled: bool,
}

fn default_policy_enabled() -> bool {
    true
}

/// `GET /policies` — list all authorization policies.
async fn list_policies_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.list_policies().await {
        Ok(policies) => Json(json!({"policies": policies})).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `GET /policies/:id` — fetch one authorization policy.
async fn get_policy_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.get_policy(&id).await {
        Ok(Some(policy)) => Json(json!(policy)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `POST /policies` — create/upsert a policy (id in the body).
async fn create_policy_handler(
    State(state): State<AdminState>,
    Json(payload): Json<UpsertPolicyRequest>,
) -> impl IntoResponse {
    let id = match payload.id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing policy id"})),
            )
                .into_response();
        }
    };
    upsert_policy_inner(&state, &id, payload).await
}

/// `PUT /policies/:id` — update/upsert a policy (id from the path).
async fn update_policy_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
    Json(payload): Json<UpsertPolicyRequest>,
) -> impl IntoResponse {
    upsert_policy_inner(&state, &id, payload).await
}

/// Shared create/update path: VALIDATE (Cedar) → persist → reload the engine.
///
/// Write-time validation is the gate: an invalid policy returns 400 with the
/// Cedar error and is NEVER written, so it can neither reach the DB nor the hot
/// path. After a successful write the engine is reloaded parse-before-swap; a
/// reload failure there does not undo the (already-validated) write but is
/// surfaced so operators can see the bundle did not advance.
async fn upsert_policy_inner(
    state: &AdminState,
    id: &str,
    payload: UpsertPolicyRequest,
) -> axum::response::Response {
    let Some(db) = &state.db else {
        return db_not_configured();
    };

    // 1. Write-time Cedar validation — reject bad policy (text, schema, AND
    // entities) BEFORE it is stored. This is a true superset of what the loader
    // can fail on, so a validated write always compiles on reload.
    let record = PolicyRecord {
        id: id.to_string(),
        policy_text: payload.policy_text.clone(),
        schema_json: payload.schema_json.clone(),
        entities_json: payload.entities_json.clone(),
    };
    if let Err(e) = validate_policy(&record) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_policy", "message": e.to_string()})),
        )
            .into_response();
    }

    // Advisory (non-blocking) breadth warnings, e.g. an allow-all permit.
    let warnings = policy_warnings(&record);

    // 2. Persist (parameterized upsert).
    if let Err(e) = db
        .upsert_policy(
            id,
            &payload.policy_text,
            payload.schema_json.as_ref(),
            payload.entities_json.as_ref(),
            payload.enabled,
        )
        .await
    {
        return internal_error(&e.to_string());
    }

    // 3. Reload the live engine (parse-before-swap, lenient, retains last-good
    // on a DB-load failure). If the reload could not run, the policy is stored
    // but NOT active on this replica — surface that as a 500 so a non-loading
    // bundle can't ship silently (H1).
    match state.authz.reload_from_database(db).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"status": "ok", "id": id, "reloaded": true, "warnings": warnings})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "stored_but_not_activated",
                "message": format!("policy stored but engine reload failed: {e}"),
                "id": id,
                "reloaded": false,
                "warnings": warnings,
            })),
        )
            .into_response(),
    }
}

/// `DELETE /policies/:id` — delete a policy, then reload the engine.
async fn delete_policy_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.delete_policy(&id).await {
        Ok(true) => match state.authz.reload_from_database(db).await {
            Ok(()) => {
                Json(json!({"status": "deleted", "id": id, "reloaded": true})).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "deleted_but_not_reloaded",
                    "message": format!("policy deleted but engine reload failed: {e}"),
                    "id": id,
                    "reloaded": false,
                })),
            )
                .into_response(),
        },
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// Standard 501 response when no database is configured.
fn db_not_configured() -> axum::response::Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({"error": "database not configured"})),
    )
        .into_response()
}

/// Standard 500 response carrying an error message.
fn internal_error(msg: &str) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": msg})),
    )
        .into_response()
}
