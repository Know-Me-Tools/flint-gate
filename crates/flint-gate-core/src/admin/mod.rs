/// Admin API — internal-only Axum router on port 4457.
///
/// Endpoints:
/// - `GET  /health` — liveness probe
/// - `GET  /ready`  — readiness probe (checks DB)
/// - `GET  /cache/stats` — cache entry counts
/// - `POST /cache/invalidate` — manual cache flush
/// - `GET  /routes` — list all routes
/// - `POST /routes` — create/update a route
/// - `GET  /routes/{id}` — get a route
/// - `PUT  /routes/{id}` — update a route
/// - `DELETE /routes/{id}` — delete a route
/// - `GET  /api-keys` — list active API keys (metadata only)
/// - `POST /api-keys` — create a new API key (returns raw key once)
/// - `DELETE /api-keys/{id}` — revoke an API key
/// - `POST /approvals/{id}/decision` — resolve a pending human-in-the-loop approval
pub mod auth;

use crate::approval::{ApprovalDecision, ApprovalError};
use crate::authz::{policy_warnings, validate_policy, AuthzEngine, PolicyRecord};
use crate::cache::GateCache;
use crate::config::SharedConfig;
use crate::db::{AuditQuery, AuthzAuditDecision, Database};
use crate::proxy::SharedRouter;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use rust_embed::RustEmbed;
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
    /// Shared human-in-the-loop approval routing table.
    pub approval_manager: crate::approval::ApprovalManager,
}

/// Build the admin Axum router.
/// Build the admin router with **no** authentication (loopback-dev posture).
pub fn admin_router(state: AdminState) -> Router {
    admin_router_with_auth(state, None)
}

/// Build the admin router, optionally protecting every route except the
/// liveness/readiness probes with the admin-auth middleware.
///
/// When `authenticator` is `Some`, the state-changing / data / analytics routes
/// and the SPA static fallback require authentication; `/health` and `/ready`
/// stay open so orchestrators can probe an authed deployment. When `None`, the
/// whole router is unauthenticated — only valid on a loopback bind, which the
/// startup posture guard enforces.
pub fn admin_router_with_auth(
    state: AdminState,
    authenticator: Option<auth::AdminAuthenticator>,
) -> Router {
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

    // Probes stay unauthenticated so liveness/readiness works on an authed
    // deployment. Everything else is a candidate for the auth layer.
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .with_state(state.clone());

    let mut protected = Router::new()
        .route("/cache/stats", get(cache_stats_handler))
        .route("/cache/invalidate", post(cache_invalidate_handler))
        .route(
            "/routes",
            get(list_routes_handler).post(upsert_route_handler),
        )
        .route(
            "/routes/{id}",
            get(get_route_handler)
                .put(upsert_route_handler_with_id)
                .delete(delete_route_handler),
        )
        .route(
            "/api-keys",
            get(list_api_keys_handler).post(create_api_key_handler),
        )
        .route(
            "/api-keys/{id}",
            axum::routing::delete(revoke_api_key_handler),
        )
        .route("/config", get(config_handler))
        .route(
            "/signing-keys",
            get(list_signing_keys_handler).post(create_signing_key_handler),
        )
        .route(
            "/signing-keys/{id}",
            axum::routing::delete(deactivate_signing_key_handler),
        )
        .route(
            "/policies",
            get(list_policies_handler).post(create_policy_handler),
        )
        .route(
            "/policies/{id}",
            get(get_policy_handler)
                .put(update_policy_handler)
                .delete(delete_policy_handler),
        )
        .route("/audit", get(list_audit_handler))
        .route("/analytics/summary", get(usage_summary_handler))
        .route("/analytics/tokens", get(token_analytics_handler))
        .route(
            "/agent-identities",
            get(list_agent_identities_handler).post(issue_agent_identity_handler),
        )
        .route(
            "/agent-identities/{id}/rotate",
            post(rotate_agent_identity_handler),
        )
        .route(
            "/agent-identities/{id}",
            axum::routing::delete(revoke_agent_identity_handler),
        )
        .route("/approvals/{id}/decision", post(decide_approval_handler))
        .merge(SwaggerUi::new("/docs").url("/openapi.json", AdminApiDoc::openapi()))
        .fallback(static_handler)
        .with_state(state);

    // Apply the auth layer to the protected sub-router only. The middleware
    // carries the authenticator as its own state.
    if let Some(auth) = authenticator {
        protected = protected.layer(axum::middleware::from_fn_with_state(
            auth,
            auth::require_admin_auth,
        ));
    }

    public.merge(protected)
}

/// `GET /health` — always 200.
#[utoipa::path(get, path = "/health", responses((status = 200, description = "Service healthy")))]
async fn health_handler() -> impl IntoResponse {
    Json(json!({"status": "ok", "service": "flint-gate"}))
}

/// Embedded admin web UI assets. In release builds they are compiled in; in
/// debug builds `debug-embed` reads from `../../web/dist` on each request so
/// `cargo run` sees the latest `pnpm build` without recompiling Rust.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct AdminAssets;

/// Resolve a request path to an embedded asset, applying the SPA fallback rule:
/// serve the exact asset when it exists, otherwise serve `index.html` so
/// client-side (history-API) routing works on deep links. Returns the resolved
/// file plus the path to derive its MIME type from. `None` means neither the
/// asset nor `index.html` is embedded (assets never built).
///
/// Pure over the embedded filesystem so the fallback decision is unit-testable
/// without constructing the full admin router or its heavy state.
fn resolve_asset(path: &str) -> Option<(rust_embed::EmbeddedFile, String)> {
    match AdminAssets::get(path) {
        Some(f) => Some((f, path.to_string())),
        None => AdminAssets::get("index.html").map(|f| (f, "index.html".to_string())),
    }
}

/// Fallback handler for the admin SPA: serves an exact embedded asset when it
/// exists, otherwise falls back to `index.html` so client-side routing works.
async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let Some((file, mime_path)) = resolve_asset(path) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Admin UI assets not built; run `pnpm build` in web/",
        )
            .into_response();
    };

    let mut response = Response::new(Body::from(file.data));
    let mime = mime_guess::from_path(&mime_path).first_or_octet_stream();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
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

/// `PUT /routes/{id}` — update route with explicit ID.
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

/// `GET /routes/{id}`
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

/// `DELETE /routes/{id}`
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

/// `DELETE /api-keys/{id}` — revoke (soft-delete) an API key.
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

/// `GET /config` — return the current loaded configuration (read-only).
async fn config_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let cfg = state.config.read().await;
    Json(cfg.clone())
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

/// `DELETE /signing-keys/{id}` — deactivate a signing key.
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

/// `GET /policies/{id}` — fetch one authorization policy.
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

/// `PUT /policies/{id}` — update/upsert a policy (id from the path).
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

/// `DELETE /policies/{id}` — delete a policy, then reload the engine.
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

// ── Authorization audit trail (read-only) ────────────────────────────────────

/// Default page size for `GET /audit` when `limit` is omitted.
const AUDIT_DEFAULT_LIMIT: i64 = 100;
/// Hard cap on `GET /audit` page size — a larger `limit` is clamped down.
const AUDIT_MAX_LIMIT: i64 = 1000;

/// Raw query parameters for `GET /audit`. All optional; parsed/clamped into an
/// [`AuditQuery`] by [`build_audit_query`].
#[derive(Debug, Deserialize)]
struct AuditParams {
    /// Exact-match principal filter.
    principal: Option<String>,
    /// Decision filter (`allow` | `deny` | `step_up` | `approval`).
    decision: Option<String>,
    /// Inclusive lower bound on `created_at` (RFC3339).
    since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `created_at` (RFC3339).
    until: Option<DateTime<Utc>>,
    /// Page size (default 100, capped at 1000, floored at 1).
    limit: Option<i64>,
    /// Row offset (default 0, floored at 0).
    offset: Option<i64>,
}

/// Convert raw `/audit` query params into a validated, clamped [`AuditQuery`].
///
/// - `limit`: absent → 100; otherwise clamped to `[1, 1000]`.
/// - `offset`: absent or negative → 0.
/// - `decision`: an unknown value is rejected (`Err`) so a typo is a 400 rather
///   than a silent empty result; absence disables the filter.
///
/// Pure and side-effect free so the clamping/validation is unit-testable without
/// a database. Returns `Err(message)` on an invalid `decision`.
fn build_audit_query(params: AuditParams) -> Result<AuditQuery, String> {
    let decision = match params.decision.as_deref() {
        None => None,
        Some(s) => Some(
            AuthzAuditDecision::parse(s)
                .ok_or_else(|| format!("invalid decision filter: {s:?}"))?,
        ),
    };

    let limit = params
        .limit
        .map(|l| l.clamp(1, AUDIT_MAX_LIMIT))
        .unwrap_or(AUDIT_DEFAULT_LIMIT);
    let offset = params.offset.map(|o| o.max(0)).unwrap_or(0);

    Ok(AuditQuery {
        principal: params.principal,
        decision,
        since: params.since,
        until: params.until,
        limit,
        offset,
    })
}

/// `GET /audit` — read-only, paged, filterable authorization audit trail.
///
/// Query params: `principal`, `decision`, `since`, `until`, `limit`, `offset`.
/// Returns `{"audit": [ … ]}` ordered newest-first. Lives on the private admin
/// router only — never the public proxy.
async fn list_audit_handler(
    State(state): State<AdminState>,
    Query(params): Query<AuditParams>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    let query = match build_audit_query(params) {
        Ok(q) => q,
        Err(msg) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response();
        }
    };
    match db.list_authz_audit(&query).await {
        Ok(rows) => Json(json!({"audit": rows})).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

// ── Analytics (read-only) ───────────────────────────────────────────────────

/// Query parameters shared by the analytics endpoints.
#[derive(Debug, Deserialize)]
struct AnalyticsParams {
    /// Inclusive lower bound on `created_at` (RFC3339).
    since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `created_at` (RFC3339).
    until: Option<DateTime<Utc>>,
    /// Bucketing interval for the time series (`hour` or `day`).
    #[serde(default = "default_analytics_interval")]
    interval: String,
    /// Maximum number of top routes/users to return (default 10, cap 100).
    #[serde(default = "default_analytics_limit")]
    limit: i64,
}

fn default_analytics_interval() -> String {
    "day".to_string()
}

fn default_analytics_limit() -> i64 {
    10
}

/// Hard cap on the analytics top-N `limit`.
const ANALYTICS_MAX_LIMIT: i64 = 100;

/// Normalize a raw analytics interval to the supported bucketing whitelist.
/// Anything other than `hour` falls back to `day`. Pure so the whitelist is
/// unit-testable and cannot drift from the DB `date_trunc` whitelist.
fn normalize_interval(raw: &str) -> &'static str {
    match raw {
        "hour" => "hour",
        _ => "day",
    }
}

/// Clamp a raw analytics `limit` to `[1, ANALYTICS_MAX_LIMIT]`.
fn clamp_analytics_limit(raw: i64) -> i64 {
    raw.clamp(1, ANALYTICS_MAX_LIMIT)
}

/// `GET /analytics/summary` — aggregate token/request/duration statistics.
async fn usage_summary_handler(
    State(state): State<AdminState>,
    Query(params): Query<AnalyticsParams>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.usage_summary(params.since, params.until).await {
        Ok(summary) => Json(json!({"summary": summary})).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `GET /analytics/tokens` — time series + top routes + top users.
async fn token_analytics_handler(
    State(state): State<AdminState>,
    Query(params): Query<AnalyticsParams>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    let interval = normalize_interval(&params.interval);
    let limit = clamp_analytics_limit(params.limit);

    match tokio::try_join!(
        db.usage_timeseries(params.since, params.until, interval),
        db.usage_by_route(params.since, params.until, limit),
        db.usage_by_user(params.since, params.until, limit),
    ) {
        Ok((timeseries, by_route, by_user)) => Json(json!({
            "interval": interval,
            "timeseries": timeseries,
            "by_route": by_route,
            "by_user": by_user,
        }))
        .into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

// ── Human-in-the-loop approvals ─────────────────────────────────────────────

/// Decision payload for `POST /approvals/{id}/decision`.
#[derive(Debug, Deserialize)]
struct ApprovalDecisionRequest {
    decision: ApprovalDecision,
}

/// `POST /approvals/{id}/decision` — approve or deny a pending approval request.
///
/// The decision is routed back to the stream task that owns the paused tool
/// call. A missing id returns 404; an expired request returns 410.
async fn decide_approval_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
    Json(payload): Json<ApprovalDecisionRequest>,
) -> impl IntoResponse {
    match state.approval_manager.decide(&id, payload.decision) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "approval_id": id,
                "decision": payload.decision,
            })),
        )
            .into_response(),
        Err(ApprovalError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "approval request not found"})),
        )
            .into_response(),
        Err(ApprovalError::Expired) => (
            StatusCode::GONE,
            Json(json!({"error": "approval request expired"})),
        )
            .into_response(),
    }
}

// ── NHI lifecycle (agent / service identities) ──────────────────────────────

/// Payload for `POST /agent-identities` — issue a non-human identity.
#[derive(Debug, Deserialize)]
struct IssueAgentIdentityRequest {
    id: String,
    /// `"agent"` or `"service"`.
    kind: String,
    #[serde(default)]
    label: Option<String>,
}

/// Write an NHI lifecycle event to the authz audit trail (best-effort).
async fn audit_nhi_event(db: &Database, id: &str, action: &str) {
    let record = crate::db::AuthzAuditRecord {
        request_id: None,
        principal: id.to_string(),
        action: action.to_string(),
        resource: "agent_identity".to_string(),
        // Lifecycle events are administrative; record as `allow` (the action
        // succeeded) — the decision column tracks authz outcomes, and these rows
        // are distinguished by their `action` (issue/rotate/revoke).
        decision: crate::db::AuthzAuditDecision::Allow,
        reason: Some(format!("nhi {action}")),
        context: Some(json!({ "agent_id": id })),
    };
    if let Err(e) = db.log_authz_decision(&record).await {
        tracing::warn!(error = %e, id, action, "nhi lifecycle audit write failed (ignored)");
    }
}

/// `GET /agent-identities` — list all non-human identities.
async fn list_agent_identities_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.list_agent_identities().await {
        Ok(items) => Json(json!({ "agent_identities": items })).into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `POST /agent-identities` — issue (register) a non-human identity.
async fn issue_agent_identity_handler(
    State(state): State<AdminState>,
    Json(payload): Json<IssueAgentIdentityRequest>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    if !matches!(payload.kind.as_str(), "agent" | "service") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "kind must be 'agent' or 'service'"})),
        )
            .into_response();
    }
    match db
        .issue_agent_identity(&payload.id, &payload.kind, payload.label.as_deref())
        .await
    {
        Ok(()) => {
            audit_nhi_event(db, &payload.id, "nhi_issue").await;
            (
                StatusCode::CREATED,
                Json(json!({"status": "issued", "id": payload.id, "kind": payload.kind})),
            )
                .into_response()
        }
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `POST /agent-identities/{id}/rotate` — stamp an identity rotated.
async fn rotate_agent_identity_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.rotate_agent_identity(&id).await {
        Ok(true) => {
            audit_nhi_event(db, &id, "nhi_rotate").await;
            Json(json!({"status": "rotated", "id": id})).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not found or not active"})),
        )
            .into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `DELETE /agent-identities/{id}` — revoke a non-human identity. Takes effect
/// on the identity's next authorize (fail-closed).
async fn revoke_agent_identity_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.revoke_agent_identity(&id).await {
        Ok(true) => {
            audit_nhi_event(db, &id, "nhi_revoke").await;
            Json(json!({"status": "revoked", "id": id})).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not found or already revoked"})),
        )
            .into_response(),
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

#[cfg(test)]
mod tests {
    use super::{
        build_audit_query, default_analytics_interval, default_analytics_limit, AuditParams,
        AUDIT_DEFAULT_LIMIT, AUDIT_MAX_LIMIT,
    };
    use crate::db::AuthzAuditDecision;

    fn params() -> AuditParams {
        AuditParams {
            principal: None,
            decision: None,
            since: None,
            until: None,
            limit: None,
            offset: None,
        }
    }

    #[test]
    fn audit_query_applies_defaults_when_params_absent() {
        let q = build_audit_query(params()).expect("valid");
        assert_eq!(q.limit, AUDIT_DEFAULT_LIMIT);
        assert_eq!(q.offset, 0);
        assert!(q.principal.is_none());
        assert!(q.decision.is_none());
    }

    #[test]
    fn audit_query_clamps_limit_to_cap_and_floor() {
        let over = build_audit_query(AuditParams {
            limit: Some(10_000),
            ..params()
        })
        .expect("valid");
        assert_eq!(over.limit, AUDIT_MAX_LIMIT);

        let under = build_audit_query(AuditParams {
            limit: Some(0),
            ..params()
        })
        .expect("valid");
        assert_eq!(under.limit, 1);

        let negative = build_audit_query(AuditParams {
            limit: Some(-5),
            ..params()
        })
        .expect("valid");
        assert_eq!(negative.limit, 1);
    }

    #[test]
    fn audit_query_floors_negative_offset_to_zero() {
        let q = build_audit_query(AuditParams {
            offset: Some(-1),
            ..params()
        })
        .expect("valid");
        assert_eq!(q.offset, 0);
    }

    #[test]
    fn audit_query_parses_known_decision_filter() {
        let q = build_audit_query(AuditParams {
            decision: Some("deny".to_string()),
            ..params()
        })
        .expect("valid");
        assert_eq!(q.decision, Some(AuthzAuditDecision::Deny));
    }

    #[test]
    fn audit_query_rejects_unknown_decision_filter() {
        let err = build_audit_query(AuditParams {
            decision: Some("banana".to_string()),
            ..params()
        })
        .expect_err("must reject unknown decision");
        assert!(err.contains("invalid decision"));
    }

    #[test]
    fn analytics_interval_defaults_to_day() {
        assert_eq!(default_analytics_interval(), "day");
    }

    #[test]
    fn analytics_limit_defaults_to_ten() {
        assert_eq!(default_analytics_limit(), 10);
    }

    // ── SPA static-asset fallback ────────────────────────────────────────────

    #[test]
    fn resolve_asset_serves_index_for_unknown_client_route() {
        // A deep client-side route (history-API path) is never an embedded file;
        // the SPA fallback must resolve it to index.html so the SPA can boot and
        // route in the browser, rather than 404.
        let resolved = super::resolve_asset("routes/my-route-id");
        let (_, mime_path) = resolved.expect("index.html fallback must resolve");
        assert_eq!(mime_path, "index.html");
    }

    #[test]
    fn resolve_asset_serves_index_itself() {
        let (_, mime_path) =
            super::resolve_asset("index.html").expect("index.html must be embedded");
        assert_eq!(mime_path, "index.html");
    }

    #[test]
    fn resolve_asset_root_path_falls_back_to_index() {
        // The trimmed root path is the empty string, which is not an asset key;
        // it must still resolve to index.html.
        let (_, mime_path) = super::resolve_asset("").expect("root must resolve to index.html");
        assert_eq!(mime_path, "index.html");
    }

    // ── Analytics interval / limit normalization ─────────────────────────────

    #[test]
    fn analytics_interval_whitelist_accepts_hour_and_day() {
        assert_eq!(super::normalize_interval("hour"), "hour");
        assert_eq!(super::normalize_interval("day"), "day");
    }

    #[test]
    fn analytics_interval_rejects_unknown_and_falls_back_to_day() {
        // A non-whitelisted interval must never reach the SQL date_trunc; it is
        // coerced to the safe default rather than passed through.
        assert_eq!(super::normalize_interval("minute"), "day");
        assert_eq!(super::normalize_interval("'; DROP TABLE"), "day");
        assert_eq!(super::normalize_interval(""), "day");
    }

    #[test]
    fn analytics_limit_clamps_to_cap_and_floor() {
        assert_eq!(super::clamp_analytics_limit(50), 50);
        assert_eq!(super::clamp_analytics_limit(10_000), super::ANALYTICS_MAX_LIMIT);
        assert_eq!(super::clamp_analytics_limit(0), 1);
        assert_eq!(super::clamp_analytics_limit(-9), 1);
    }
}
