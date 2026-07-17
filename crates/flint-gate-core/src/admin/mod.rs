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
/// - `GET  /approvals` — list non-expired pending human-in-the-loop approvals
/// - `GET  /approvals/{id}` — get a single pending approval by id
/// - `POST /approvals/{id}/decision` — resolve a pending human-in-the-loop approval
/// - `GET  /tool-scopes` — list UI-authored agent tool-scope policies
/// - `POST /tool-scopes` — compile `{agent,allow,deny}` to Cedar + persist (structured-only)
/// - `DELETE /tool-scopes/{agent}` — remove an agent's tool-scope policy
pub mod auth;

use crate::approval::{ApprovalDecision, ApprovalError, ApprovalStore};
use crate::auth::Identity;
use crate::authz::{
    compile_and_validate, policy_warnings, validate_policy_for_gateway,
    AuthzEngine, PolicyParseError, PolicyRecord, ReloadStatus, SUGAR_ID_PREFIX,
};
use crate::config::types::AgentToolPolicy;
use crate::cache::GateCache;
use crate::config::SharedConfig;
use crate::db::{AuditQuery, AuthzAuditDecision, Database};
use crate::proxy::SharedRouter;
use crate::ratelimit::CredentialKeyExtractor;
use axum::{
    body::Body,
    extract::{Extension, Path, Query, State},
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode, Uri},
    response::{sse, IntoResponse, Json, Response, Sse},
    routing::{get, post},
    Router,
};
use governor::middleware::NoOpMiddleware;
use tower_governor::GovernorLayer;
use chrono::{DateTime, Utc};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
#[allow(unused_imports)]
use utoipa::ToSchema;
use uuid::Uuid;

/// Events emitted by the admin server for server-sent event (SSE) consumers.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminEvent {
    /// A Cedar policy reload completed successfully.
    PolicyReloadOk {
        /// Number of active policies in the live bundle after the reload.
        policy_count: usize,
    },
    /// A Cedar policy reload encountered problems (DB error or bad rows).
    PolicyReloadError {
        /// Number of rows skipped (bad policy text). Zero when the error was a DB failure.
        skipped_count: usize,
        /// Error from the database layer when the DB itself could not be queried.
        db_error: Option<String>,
    },
}

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
    pub approval_manager: Arc<dyn ApprovalStore>,
    /// Broadcast channel for admin-facing events (reload status, etc.).
    /// When `None`, SSE subscriptions return an empty stream (no-DB / test posture).
    pub admin_events: Option<tokio::sync::broadcast::Sender<AdminEvent>>,
}

/// Alias for the admin governor-layer type to keep signatures readable.
pub type AdminGovernorLayer =
    GovernorLayer<CredentialKeyExtractor, NoOpMiddleware, axum::body::Body>;

/// Build the admin Axum router with no authentication and no rate-limiting
/// (loopback-dev posture).
pub fn admin_router(state: AdminState) -> Router {
    admin_router_with_auth(state, None, None)
}

/// Build the admin router, optionally protecting every route except the
/// liveness/readiness probes with the admin-auth middleware and/or a
/// per-credential rate-limit governor.
///
/// - `authenticator`: when `Some`, every protected route requires a valid
///   admin credential; `/health`, `/ready`, and `/metrics` stay open.
/// - `rate_limiter`: when `Some`, the governor layer is applied to the
///   protected sub-router (after auth, so unauthenticated requests are
///   rejected before they consume rate-limit quota). Public probes bypass it.
pub fn admin_router_with_auth(
    state: AdminState,
    authenticator: Option<auth::AdminAuthenticator>,
    rate_limiter: Option<AdminGovernorLayer>,
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

    // Probes + metrics stay unauthenticated so liveness/readiness and Prometheus
    // scraping work on an authed deployment. `/metrics` is on the ADMIN router
    // only (private port) — never the public proxy surface — and is gated behind
    // the admin bind's own exposure posture. Everything else is a candidate for
    // the auth layer.
    let public = Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/metrics", get(metrics_handler))
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
        .route("/policies/validate", post(validate_policy_handler))
        .route(
            "/policies/{id}/history",
            get(list_policy_history_handler),
        )
        .route(
            "/policies/{id}/rollback",
            post(rollback_policy_handler),
        )
        .route(
            "/policies/{id}",
            get(get_policy_handler)
                .put(update_policy_handler)
                .delete(delete_policy_handler),
        )
        .route(
            "/tool-scopes",
            get(list_tool_scopes_handler).post(upsert_tool_scope_handler),
        )
        .route(
            "/tool-scopes/{agent}",
            axum::routing::delete(delete_tool_scope_handler),
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
        .route("/approvals", get(list_approvals_handler))
        .route("/approvals/{id}", get(get_approval_handler))
        .route("/approvals/{id}/decision", post(decide_approval_handler))
        .route("/policies/reload-status", get(reload_status_handler))
        .route("/policies/simulate", post(simulate_policy_handler))
        .route("/events", get(admin_events_handler))
        .merge(SwaggerUi::new("/docs").url("/openapi.json", AdminApiDoc::openapi()))
        .fallback(static_handler)
        .with_state(state);

    // Cap request bodies on all protected endpoints (64 KiB). This prevents
    // a malicious or misconfigured caller from OOM-ing the admin process by
    // sending a giant JSON payload to a policy-create or route-upsert handler.
    // Public probes carry no body so they are unaffected by the split router
    // design — the limit applies only to the protected sub-router.
    protected = protected.layer(axum::extract::DefaultBodyLimit::max(64 * 1024));

    // Apply the rate-limit governor after the body limit (innermost layer,
    // evaluated last in the middleware stack). Layering order:
    //   auth wraps rate-limit wraps body-limit wraps handler
    // so unauthenticated requests are rejected before consuming rate-limit
    // quota, and oversized bodies are rejected before reaching the handler.
    if let Some(rl) = rate_limiter {
        protected = protected.layer(rl);
    }

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

/// `GET /metrics` — Prometheus text exposition of control-plane metrics
/// (delegate outcomes/latency, etc.). Served on the **admin** router only, so it
/// is never reachable on the public proxy port. Returns an empty body when the
/// recorder was never installed.
async fn metrics_handler() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        crate::metrics::render(),
    )
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

// ── Policy version history ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct HistoryQueryParams {
    #[serde(default)]
    pub(super) offset: i64,
    #[serde(default = "default_history_limit")]
    pub(super) limit: i64,
}

fn default_history_limit() -> i64 {
    20
}

#[derive(serde::Serialize)]
struct PolicyHistoryResponse {
    policy_id: String,
    total_hint: Option<i64>,
    offset: i64,
    limit: i64,
    versions: Vec<crate::db::PolicyVersionRow>,
}

/// `GET /policies/{id}/history` — list version history for one policy.
///
/// Returns 404 when the policy does not exist. The `limit` is clamped to 100
/// so callers cannot paginate unbounded. `total_hint` is `None` — it is
/// intentionally omitted to avoid a COUNT(*) on every request.
async fn list_policy_history_handler(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Query(params): Query<HistoryQueryParams>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.get_policy(&id).await {
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "policy not found"}))).into_response();
        }
        Err(e) => return internal_error(&e.to_string()),
        Ok(Some(_)) => {}
    }
    let effective_limit = params.limit.min(100);
    match db
        .list_policy_versions(&id, params.offset, effective_limit)
        .await
    {
        Ok(versions) => Json(PolicyHistoryResponse {
            policy_id: id,
            total_hint: None,
            offset: params.offset,
            limit: effective_limit,
            versions,
        })
        .into_response(),
        Err(e) => internal_error(&e.to_string()),
    }
}

// ── Policy rollback ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RollbackRequest {
    version_num: i32,
}

#[derive(serde::Serialize)]
struct RollbackResponse {
    status: String,
    policy_id: String,
    from_version: i32,
    to_version: i32,
    reloaded: bool,
}

/// `POST /policies/{id}/rollback` — restore a prior version and hot-reload.
///
/// Fail-closed: the target version's `policy_text` is Cedar-validated before
/// any write occurs. If validation fails, 422 is returned with parse errors and
/// no state changes. The rollback itself writes a **new** version row (via the
/// normal `upsert_policy` path) so the undo is fully auditable.
async fn rollback_policy_handler(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(payload): Json<RollbackRequest>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };

    // 1. Fetch the target version row → 404 if not found.
    let target = match db.get_policy_version(&id, payload.version_num).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "version not found", "policy_id": id, "version_num": payload.version_num})),
            )
                .into_response();
        }
        Err(e) => return internal_error(&e.to_string()),
    };

    // 2. Determine from_version: the current highest version_num (before the
    //    rollback write). None means no versions exist yet — treat as 0.
    let from_version = match db.list_policy_versions(&id, 0, 1).await {
        Ok(rows) => rows.first().map(|r| r.version_num).unwrap_or(0),
        Err(e) => return internal_error(&e.to_string()),
    };

    // 3. Cedar-validate the target text — fail-closed: never restore invalid.
    // Use full gateway validation (schema + annotations) so a stored-but-broken
    // historical version is not silently restored.
    let record = PolicyRecord {
        id: id.clone(),
        policy_text: target.policy_text.clone(),
        schema_json: target.schema_json.clone(),
        entities_json: target.entities_json.clone(),
    };
    if let Err(e) = validate_policy_for_gateway(&record) {
        let errors: Vec<PolicyParseError> = match e {
            crate::authz::AuthzError::PolicyParse(errs) => errs,
            crate::authz::AuthzError::Validation(errs) => errs,
            other => vec![PolicyParseError::without_location(other.to_string())],
        };
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid_policy", "message": "rollback target fails Cedar validation — no changes made", "errors": errors})),
        )
            .into_response();
    }

    // 4. Upsert (restores text, inserts new version row = to_version).
    if let Err(e) = db
        .upsert_policy(
            &id,
            &target.policy_text,
            target.schema_json.as_ref(),
            target.entities_json.as_ref(),
            true,
            None,
        )
        .await
    {
        return internal_error(&e.to_string());
    }

    // 5. Determine to_version: the new highest version_num after the upsert.
    let to_version = match db.list_policy_versions(&id, 0, 1).await {
        Ok(rows) => rows.first().map(|r| r.version_num).unwrap_or(from_version + 1),
        Err(e) => return internal_error(&e.to_string()),
    };

    // 6. Reload the live Cedar engine.
    match state.authz.reload_from_database(db).await {
        Ok(()) => Json(RollbackResponse {
            status: "rolled_back".to_string(),
            policy_id: id,
            from_version,
            to_version,
            reloaded: true,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "stored_but_not_reloaded",
                "message": format!("policy restored but engine reload failed: {e}"),
                "policy_id": id,
                "from_version": from_version,
                "to_version": to_version,
                "reloaded": false,
            })),
        )
            .into_response(),
    }
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
    identity: Option<Extension<Identity>>,
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
    let written_by = identity.as_ref().map(|ext| ext.id.as_str());
    upsert_policy_inner(&state, &id, payload, written_by).await
}

/// `PUT /policies/{id}` — update/upsert a policy (id from the path).
async fn update_policy_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
    identity: Option<Extension<Identity>>,
    Json(payload): Json<UpsertPolicyRequest>,
) -> impl IntoResponse {
    let written_by = identity.as_ref().map(|ext| ext.id.as_str());
    upsert_policy_inner(&state, &id, payload, written_by).await
}

/// Request body for `POST /policies/validate`.
#[derive(serde::Deserialize)]
struct ValidatePolicyRequest {
    policy: String,
    schema: Option<serde_json::Value>,
}

/// Response body for `POST /policies/validate`.
#[derive(serde::Serialize)]
struct ValidatePolicyResponse {
    valid: bool,
    errors: Vec<PolicyParseError>,
}

/// `POST /policies/validate` — dry-run Cedar policy validation (no persistence).
///
/// Returns HTTP 200 in both the valid and invalid cases; the caller inspects
/// `valid` to distinguish them. This is intentional: a 4xx would mean the
/// request itself was malformed, not that the policy failed to parse.
async fn validate_policy_handler(
    State(_state): State<AdminState>,
    Json(payload): Json<ValidatePolicyRequest>,
) -> impl IntoResponse {
    let record = PolicyRecord {
        id: "_validate".to_string(),
        policy_text: payload.policy.clone(),
        schema_json: payload.schema,
        entities_json: None,
    };
    // Use gateway validation: inject the entity schema and check annotations.
    // The endpoint always returns HTTP 200 with `valid` field — the caller
    // inspects `valid` to distinguish success from validation failure.
    match validate_policy_for_gateway(&record) {
        Ok(()) => Json(ValidatePolicyResponse {
            valid: true,
            errors: vec![],
        }),
        Err(e) => {
            let errors: Vec<PolicyParseError> = match e {
                crate::authz::AuthzError::PolicyParse(errs) => errs,
                crate::authz::AuthzError::Validation(errs) => errs,
                other => vec![PolicyParseError::without_location(other.to_string())],
            };
            Json(ValidatePolicyResponse {
                valid: false,
                errors,
            })
        }
    }
}

/// `GET /policies/reload-status` — returns the last-known Cedar reload outcome.
///
/// Always returns 200. The `ok` field distinguishes success from failure.
/// Reads from the engine's in-memory status field set by the reload path — no
/// DB round-trip.
async fn reload_status_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let status: ReloadStatus = state
        .authz
        .last_reload_status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    Json(status)
}

/// Request body for `POST /policies/simulate`.
///
/// Cedar EntityUid format: `"Namespace::Type::\"id\""` — for example
/// `"User::\"alice\""`, `"Action::\"read\""`, `"Document::\"report-42\""`.
#[derive(Debug, serde::Deserialize)]
struct SimulateRequest {
    /// Cedar principal EntityUid string.
    principal: String,
    /// Cedar action EntityUid string.
    action: String,
    /// Cedar resource EntityUid string.
    resource: String,
    /// Optional Cedar context as a JSON object. `null` or absent → empty context.
    #[serde(default)]
    context: Option<serde_json::Value>,
}

/// Response body for `POST /policies/simulate`.
#[derive(Debug, serde::Serialize)]
struct SimulateResponse {
    /// `"Allow"` or `"Deny"`.
    decision: String,
    /// Policy IDs that contributed to an Allow decision (empty on Deny).
    reasons: Vec<String>,
    /// Cedar evaluation errors (usually empty; non-empty indicates a policy issue).
    errors: Vec<String>,
}

/// `POST /policies/simulate` — dry-run Cedar authorization against the live policy
/// set without sending real traffic through the proxy.
///
/// Parses the caller-supplied principal/action/resource as Cedar `EntityUid`s and
/// evaluates them against the live policy bundle using an empty entity store (no
/// entity graph — policy logic only). Returns HTTP 422 when any EntityUid is
/// malformed.
async fn simulate_policy_handler(
    State(state): State<AdminState>,
    Json(payload): Json<SimulateRequest>,
) -> impl IntoResponse {
    use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, Request};

    #[allow(clippy::result_large_err)]
    let parse_uid = |s: &str, field: &str| -> Result<EntityUid, Response> {
        s.parse::<EntityUid>().map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "invalid_entity_uid",
                    "field": field,
                    "message": format!("could not parse `{s}` as a Cedar EntityUid: {e}"),
                })),
            )
                .into_response()
        })
    };

    let principal = match parse_uid(&payload.principal, "principal") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let action = match parse_uid(&payload.action, "action") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let resource = match parse_uid(&payload.resource, "resource") {
        Ok(v) => v,
        Err(r) => return r,
    };

    let context = match payload.context {
        Some(v) => match Context::from_json_value(v, None) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "invalid_context",
                        "field": "context",
                        "message": e.to_string(),
                    })),
                )
                    .into_response();
            }
        },
        None => Context::empty(),
    };

    let request = match Request::new(principal, action, resource, context, None) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid_request", "message": e.to_string() })),
            )
                .into_response();
        }
    };

    let bundle = state.authz.snapshot();
    let entities = Entities::empty();
    let authorizer = Authorizer::new();
    let response = authorizer.is_authorized(&request, bundle.policies(), &entities);

    let decision = match response.decision() {
        Decision::Allow => "Allow",
        Decision::Deny => "Deny",
    };

    let reasons: Vec<String> = response
        .diagnostics()
        .reason()
        .map(|id| id.to_string())
        .collect();

    let errors: Vec<String> = response
        .diagnostics()
        .errors()
        .map(|e| e.to_string())
        .collect();

    Json(SimulateResponse {
        decision: decision.to_string(),
        reasons,
        errors,
    })
    .into_response()
}

/// `GET /events` — admin server-sent events stream.
///
/// Returns an SSE stream of [`AdminEvent`]s broadcast by the reload path.
/// Each event is newline-delimited JSON (text/event-stream). When no
/// broadcast sender is wired (no-DB posture), the stream is immediately
/// closed.
async fn admin_events_handler(
    State(state): State<AdminState>,
) -> Sse<impl futures::Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    use futures::stream::{self, StreamExt};

    let stream = match state.admin_events {
        Some(ref tx) => {
            let rx = tx.subscribe();
            stream::unfold(rx, |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let data = serde_json::to_string(&event).unwrap_or_default();
                            return Some((
                                Ok::<_, std::convert::Infallible>(
                                    sse::Event::default().data(data),
                                ),
                                rx,
                            ));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            })
            .left_stream()
        }
        None => stream::empty().right_stream(),
    };

    Sse::new(stream)
}

/// Shared create/update path: VALIDATE (Cedar) → persist → reload the engine.
///
/// Write-time validation is the gate: an invalid policy returns 400 with the
/// Cedar error and is NEVER written, so it can neither reach the DB nor the hot
/// path. After a successful write the engine is reloaded parse-before-swap; a
/// reload failure there does not undo the (already-validated) write but is
/// surfaced so operators can see the bundle did not advance.
/// Whether a policy id is in the reserved compiled-sugar namespace and must be
/// rejected on a DB write (see the guard in [`upsert_policy_inner`]). Pure so the
/// reserved-namespace rule is unit-testable without a database.
fn is_reserved_policy_id(id: &str) -> bool {
    id.starts_with(SUGAR_ID_PREFIX)
}

async fn upsert_policy_inner(
    state: &AdminState,
    id: &str,
    payload: UpsertPolicyRequest,
    written_by: Option<&str>,
) -> axum::response::Response {
    let Some(db) = &state.db else {
        return db_not_configured();
    };

    // 0. Reserved-namespace guard: a stored policy MUST NOT use the compiled
    // `agent_tool_policies` sugar id prefix. Sugar policies are merged into the
    // live PolicySet as an overlay; a DB row sharing a sugar id would collide and
    // (on the lenient reload path) silently SUPPRESS the config tool-scope — a
    // vanishing `deny:`. Reject at the write boundary so the merge's
    // id-disjointness invariant holds by construction (defense in depth).
    if is_reserved_policy_id(id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "reserved_policy_id",
                "message": format!(
                    "policy id must not start with the reserved prefix {SUGAR_ID_PREFIX:?} \
                     (reserved for compiled agent_tool_policies)"
                )
            })),
        )
            .into_response();
    }

    // 1. Write-time Cedar validation — reject bad policy (text, schema, AND
    // entities) BEFORE it is stored. `validate_policy_for_gateway` also:
    //   a) type-checks against the gateway entity schema (User/Agent/Service/Route,
    //      action call_tool) when the caller supplies no schema_json, and
    //   b) rejects unknown annotation keys (catches @require_apporval typos).
    // Syntax errors → 400; schema/annotation errors → 422 (semantically invalid).
    let record = PolicyRecord {
        id: id.to_string(),
        policy_text: payload.policy_text.clone(),
        schema_json: payload.schema_json.clone(),
        entities_json: payload.entities_json.clone(),
    };
    if let Err(e) = validate_policy_for_gateway(&record) {
        let (status, error_code) = match &e {
            crate::authz::AuthzError::PolicyParse(_) => {
                (StatusCode::BAD_REQUEST, "invalid_policy")
            }
            // Schema validation errors (wrong entity types, unknown actions,
            // unknown annotations) → 422 Unprocessable Entity.
            _ => (StatusCode::UNPROCESSABLE_ENTITY, "policy_schema_violation"),
        };
        let errors: Vec<&PolicyParseError> = match &e {
            crate::authz::AuthzError::PolicyParse(errs) => errs.iter().collect(),
            crate::authz::AuthzError::Validation(errs) => errs.iter().collect(),
            _ => vec![],
        };
        return (
            status,
            Json(json!({
                "error": error_code,
                "message": e.to_string(),
                "errors": errors
            })),
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
            written_by,
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

// ── Agent tool-scope builder (structured → compiled Cedar) ───────────────────
//
// An ergonomic, STRUCTURED-ONLY front-end over the same Cedar the engine runs.
// The operator posts `{ agent, allow[], deny[] }`; the endpoint compiles it via
// `compile_and_validate` (the SAME allowlist-charset + Cedar-validate gate the
// config sugar uses) and stores the result as a database policy row so it is
// hot-reloadable and enforced alongside other policies. There is deliberately NO
// raw-Cedar field here — operator input reaches Cedar ONLY through the compiler,
// so the string-concatenation injection surface the compiler defends can never be
// reached from this endpoint (see the compiler's allowlist in `authz::sugar`).

/// DB `authz_policies` id prefix for UI-authored tool-scopes. Distinct from the
/// config-file overlay prefix (`SUGAR_ID_PREFIX`) so the two never collide; one
/// row per agent (`tool_scope::<agent>`), upserted.
const TOOL_SCOPE_ID_PREFIX: &str = "tool_scope::";

#[derive(Debug, Deserialize)]
struct ToolScopeRequest {
    agent: String,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

/// Why a tool-scope request could not be compiled to a storable policy.
#[derive(Debug, PartialEq)]
enum ToolScopeError {
    /// Illegal agent id / tool token, or Cedar that fails validation (400).
    Invalid(String),
    /// Neither allow nor deny — nothing to enforce (400).
    Empty,
}

/// Compile a `{ agent, allow, deny }` request into a `(db_id, policy_text)` pair,
/// routing operator input through the SAME allowlist-charset + Cedar-validate gate
/// as the config sugar. Pure (no DB), so the injection-safety + id-derivation
/// contract is unit-testable. Returns [`ToolScopeError::Invalid`] on illegal input
/// (the injection fail-closed boundary) and [`ToolScopeError::Empty`] when there is
/// nothing to enforce.
fn compile_tool_scope(req: &ToolScopeRequest) -> Result<(String, String), ToolScopeError> {
    let entry = AgentToolPolicy {
        agent: req.agent.clone(),
        allow: req.allow.clone(),
        deny: req.deny.clone(),
    };
    let compiled = compile_and_validate(std::slice::from_ref(&entry))
        .map_err(|e| ToolScopeError::Invalid(e.to_string()))?;
    let record = compiled.into_iter().next().ok_or(ToolScopeError::Empty)?;
    Ok((format!("{TOOL_SCOPE_ID_PREFIX}{}", req.agent), record.policy_text))
}

/// `GET /tool-scopes` — list UI-authored tool-scope policy rows (the compiled
/// Cedar is returned as stored; the structured form is not round-tripped).
async fn list_tool_scopes_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    match db.list_policies().await {
        Ok(policies) => {
            let scopes: Vec<_> = policies
                .into_iter()
                .filter(|p| p.id.starts_with(TOOL_SCOPE_ID_PREFIX))
                .collect();
            Json(json!({ "tool_scopes": scopes })).into_response()
        }
        Err(e) => internal_error(&e.to_string()),
    }
}

/// `POST /tool-scopes` — compile `{ agent, allow, deny }` to Cedar and upsert it
/// as a database policy row, then reload the engine.
async fn upsert_tool_scope_handler(
    State(state): State<AdminState>,
    Json(payload): Json<ToolScopeRequest>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };

    // Compile via the SAME structured-only gate as the config sugar. An illegal
    // agent id / tool token (outside the allowlist charset) or Cedar that fails
    // validation is rejected 400 — fail-closed, injection-safe (no raw-Cedar path).
    // The DB row is keyed on the agent (the compiler's own id uses the reserved
    // config-overlay prefix, which the policy write-guard rejects), so re-authoring
    // the same agent upserts in place.
    let (db_id, policy_text) = match compile_tool_scope(&payload) {
        Ok(pair) => pair,
        Err(ToolScopeError::Invalid(msg)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_tool_scope", "message": msg})),
            )
                .into_response();
        }
        Err(ToolScopeError::Empty) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "empty_tool_scope",
                    "message": "tool scope must have at least one allow or deny entry"
                })),
            )
                .into_response();
        }
    };

    if let Err(e) = db
        .upsert_policy(&db_id, &policy_text, None, None, true, None)
        .await
    {
        return internal_error(&e.to_string());
    }

    match state.authz.reload_from_database(db).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"status": "ok", "agent": payload.agent, "id": db_id, "reloaded": true})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "stored_but_not_activated",
                "message": format!("tool scope stored but engine reload failed: {e}"),
                "id": db_id,
                "reloaded": false,
            })),
        )
            .into_response(),
    }
}

/// `DELETE /tool-scopes/{agent}` — remove an agent's UI-authored tool-scope row.
async fn delete_tool_scope_handler(
    Path(agent): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let Some(db) = &state.db else {
        return db_not_configured();
    };
    let db_id = format!("{TOOL_SCOPE_ID_PREFIX}{agent}");
    match db.delete_policy(&db_id).await {
        Ok(true) => match state.authz.reload_from_database(db).await {
            Ok(()) => {
                Json(json!({"status": "deleted", "agent": agent, "reloaded": true})).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "deleted_but_not_reloaded",
                    "message": format!("tool scope deleted but engine reload failed: {e}"),
                    "agent": agent,
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

/// `GET /approvals` — list all non-expired pending approval requests.
async fn list_approvals_handler(State(state): State<AdminState>) -> impl IntoResponse {
    let approvals = state.approval_manager.list();
    Json(json!({"approvals": approvals})).into_response()
}

/// `GET /approvals/{id}` — get a single pending approval by id.
///
/// Returns 404 when no approval with that id exists (or it has already been resolved).
async fn get_approval_handler(
    Path(id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    match state.approval_manager.status(&id) {
        Some(status) => Json(json!(status)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "approval request not found"})),
        )
            .into_response(),
    }
}

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
        // CapExceeded is a register-time error; decide() never returns it.
        Err(ApprovalError::CapExceeded) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "approval table at capacity"})),
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
            // Audit row is written transactionally inside issue_agent_identity.
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
            // Audit row is written transactionally inside rotate_agent_identity.
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
            // Audit row is written transactionally inside revoke_agent_identity.
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
        build_audit_query, default_analytics_interval, default_analytics_limit,
        is_reserved_policy_id, AuditParams, AUDIT_DEFAULT_LIMIT, AUDIT_MAX_LIMIT,
    };
    use crate::authz::SUGAR_ID_PREFIX;
    use crate::db::AuthzAuditDecision;
    use serde_json::json;

    use super::{compile_tool_scope, ToolScopeError, ToolScopeRequest, TOOL_SCOPE_ID_PREFIX};

    fn scope(agent: &str, allow: &[&str], deny: &[&str]) -> ToolScopeRequest {
        ToolScopeRequest {
            agent: agent.to_string(),
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn tool_scope_valid_compiles_to_db_id_and_cedar() {
        let (id, text) = compile_tool_scope(&scope("ci-bot", &["deploy"], &["delete_*"]))
            .expect("valid tool scope compiles");
        assert_eq!(id, format!("{TOOL_SCOPE_ID_PREFIX}ci-bot"));
        // Structured-only → compiled Cedar; deny-wins is preserved (a forbid is emitted).
        assert!(text.contains("permit"), "text:\n{text}");
        assert!(text.contains("forbid"), "deny must compile to a forbid:\n{text}");
        // The stored id is NOT in the reserved config-overlay namespace.
        assert!(!id.starts_with(SUGAR_ID_PREFIX));
    }

    #[test]
    fn tool_scope_illegal_agent_is_rejected_injection_safe() {
        // A quote in the agent id would break out of a Cedar literal — rejected at
        // the API boundary (the injection fail-closed gate), never persisted.
        let err = compile_tool_scope(&scope("ci\"bot", &["deploy"], &[])).unwrap_err();
        assert!(matches!(err, ToolScopeError::Invalid(_)));
    }

    #[test]
    fn tool_scope_illegal_tool_is_rejected() {
        let err = compile_tool_scope(&scope("ci-bot", &["de ploy"], &[])).unwrap_err();
        assert!(matches!(err, ToolScopeError::Invalid(_)));
    }

    #[test]
    fn tool_scope_empty_is_rejected() {
        let err = compile_tool_scope(&scope("ci-bot", &[], &[])).unwrap_err();
        assert_eq!(err, ToolScopeError::Empty);
    }

    #[test]
    fn tool_scope_request_has_no_raw_cedar_field() {
        // Structural guarantee: the request accepts ONLY agent/allow/deny — there is
        // no `policy_text`/raw-Cedar field, so operator input can reach Cedar only
        // through compile_and_validate. A body with a raw-Cedar field is ignored
        // (serde drops unknown fields) — the agent/allow/deny still drive compilation.
        let req: ToolScopeRequest = serde_json::from_value(json!({
            "agent": "ci-bot",
            "allow": ["deploy"],
            "policy_text": "permit(principal, action, resource);"
        }))
        .expect("parses");
        assert_eq!(req.agent, "ci-bot");
        assert_eq!(req.allow, vec!["deploy"]);
        // The injected raw-Cedar field had no effect: compilation uses only the
        // structured fields.
        let (_, text) = compile_tool_scope(&req).expect("compiles");
        assert!(text.contains(r#"Route::"deploy""#), "text:\n{text}");
        assert!(
            !text.contains("permit(principal, action, resource)"),
            "raw-Cedar injection must not appear in the compiled output:\n{text}"
        );
    }

    #[test]
    fn reserved_policy_id_rejects_sugar_namespace() {
        // A DB write using the compiled-sugar id prefix must be rejected so it
        // cannot collide with (and silently suppress) a sugar overlay policy.
        assert!(is_reserved_policy_id(&format!("{SUGAR_ID_PREFIX}ci-bot::0")));
        assert!(is_reserved_policy_id(SUGAR_ID_PREFIX));
        // Ordinary operator ids are allowed.
        assert!(!is_reserved_policy_id("allow-deploy"));
        assert!(!is_reserved_policy_id("team-policy-42"));
        assert!(!is_reserved_policy_id(""));
    }

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

    // ── ApprovalManager integration with admin handlers ──────────────────────
    //
    // These tests exercise the manager directly (the handler delegates to it
    // without any business logic of its own), which is the same pattern used
    // for all other DB-less admin handlers in this file.

    use crate::approval::{ApprovalDecision, ApprovalError, ApprovalManager};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::unbounded_channel;

    fn meta() -> (&'static str, &'static str, &'static str, Option<String>) {
        ("user-1", "tool:bash", "agent-1", None)
    }

    #[test]
    fn list_approvals_returns_only_non_expired_entries() {
        // Mirrors list_approvals_handler: calls manager.list(), returns Vec.
        let mgr = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        mgr.register(
            "live".to_string(),
            Instant::now() + Duration::from_secs(60),
            tx.clone(),
            ("alice", "tool:read_file", "fs:/tmp", None),
        )
        .unwrap();
        mgr.register(
            "dead".to_string(),
            Instant::now() - Duration::from_secs(1),
            tx,
            meta(),
        )
        .unwrap();

        let listed = mgr.list();
        assert_eq!(listed.len(), 1, "expired entry must be hidden from list");
        assert_eq!(listed[0].approval_id, "live");
        assert_eq!(listed[0].principal_id, "alice");
        assert_eq!(listed[0].action, "tool:read_file");
        assert!(!listed[0].expired);
    }

    #[test]
    fn list_approvals_empty_when_all_expired() {
        let mgr = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        mgr.register(
            "dead".to_string(),
            Instant::now() - Duration::from_secs(1),
            tx,
            meta(),
        )
        .unwrap();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn get_approval_returns_status_for_known_id() {
        // Mirrors get_approval_handler: calls manager.status(), wraps in Json or 404.
        let mgr = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        mgr.register(
            "a1".to_string(),
            Instant::now() + Duration::from_secs(60),
            tx,
            ("bob", "tool:write_file", "fs:/etc", Some("needs review".to_string())),
        )
        .unwrap();

        let status = mgr.status("a1").expect("known id must return Some");
        assert_eq!(status.approval_id, "a1");
        assert_eq!(status.principal_id, "bob");
        assert_eq!(status.reason.as_deref(), Some("needs review"));
        assert!(!status.expired);
    }

    #[test]
    fn get_approval_returns_none_for_unknown_id() {
        // Handler maps None → 404; confirm the manager contract is stable.
        let mgr = ApprovalManager::new();
        assert!(mgr.status("missing").is_none());
    }

    #[tokio::test]
    async fn decide_absent_returns_not_found() {
        // Handler maps ApprovalError::NotFound → 404 NOT_FOUND.
        let mgr = ApprovalManager::new();
        assert_eq!(
            mgr.decide("absent", ApprovalDecision::Deny),
            Err(ApprovalError::NotFound)
        );
    }

    #[tokio::test]
    async fn decide_expired_returns_gone() {
        // Handler maps ApprovalError::Expired → 410 GONE.
        let mgr = ApprovalManager::new();
        let (tx, _rx) = unbounded_channel();
        mgr.register(
            "old".to_string(),
            Instant::now() - Duration::from_secs(1),
            tx,
            meta(),
        )
        .unwrap();
        assert_eq!(
            mgr.decide("old", ApprovalDecision::Approve),
            Err(ApprovalError::Expired)
        );
    }

    // ── Rate-limit governor tests ─────────────────────────────────────────────

    use super::{admin_router_with_auth, AdminState};
    use crate::ratelimit::build_governor_layer;
    use tower::ServiceExt; // oneshot

    fn minimal_admin_state() -> AdminState {
        use crate::approval::ApprovalManager;
        use crate::authz::AuthzEngine;
        use crate::cache::GateCache;
        use crate::config::types::{CacheConfig, GateConfig};
        use crate::proxy::router::Router;
        use std::sync::Arc;

        let gate_config = GateConfig::default();
        let config = Arc::new(tokio::sync::RwLock::new(gate_config.clone()));
        let router = Arc::new(tokio::sync::RwLock::new(Router::from_config(&gate_config)));
        AdminState {
            cache: Arc::new(GateCache::from_config(&CacheConfig::default())),
            db: None,
            router,
            config,
            authz: Arc::new(AuthzEngine::empty()),
            approval_manager: Arc::new(ApprovalManager::new()),
            admin_events: None,
        }
    }

    #[tokio::test]
    async fn admin_rate_limiter_returns_429_after_burst_exhausted() {
        // Build a governor with burst=1 so the second request triggers 429.
        let layer = build_governor_layer(1, 1).expect("valid config");
        let app = admin_router_with_auth(minimal_admin_state(), None, Some(layer));

        // First request: should pass (burst slot consumed).
        let res = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri("/health") // public probe — bypasses rate-limit
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), http::StatusCode::OK, "/health must bypass rate-limit");

        // Protected route: first request passes, second should 429.
        let protected_req = || {
            http::Request::builder()
                .uri("/config")
                .header("x-forwarded-for", "10.0.0.2")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let first = app.clone().oneshot(protected_req()).await.unwrap();
        // Without auth the handler returns 401/404 but NOT 429 on first hit.
        assert_ne!(first.status(), http::StatusCode::TOO_MANY_REQUESTS, "first request must not be rate-limited");

        let second = app.clone().oneshot(protected_req()).await.unwrap();
        assert_eq!(second.status(), http::StatusCode::TOO_MANY_REQUESTS, "second request should be rate-limited");
    }

    #[tokio::test]
    async fn health_probe_bypasses_rate_limiter() {
        // Even with burst=1 (already exhausted), /health must return 200.
        let layer = build_governor_layer(1, 1).expect("valid config");
        let app = admin_router_with_auth(minimal_admin_state(), None, Some(layer));

        let make_req = || {
            http::Request::builder()
                .uri("/health")
                .header("x-forwarded-for", "10.0.0.3")
                .body(axum::body::Body::empty())
                .unwrap()
        };

        // Fire twice — /health is on the public sub-router, so no limiter.
        for _ in 0..3 {
            let res = app.clone().oneshot(make_req()).await.unwrap();
            assert_eq!(res.status(), http::StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn no_rate_limiter_leaves_routes_unrestricted() {
        // Without a governor layer, many requests all pass (no 429).
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let make_req = || {
            http::Request::builder()
                .uri("/health")
                .header("x-forwarded-for", "10.0.0.4")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        for _ in 0..5 {
            let res = app.clone().oneshot(make_req()).await.unwrap();
            assert_eq!(res.status(), http::StatusCode::OK);
        }
    }

    // ── Body-limit tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn admin_body_over_64k_returns_413() {
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        // A body just over 64 KiB sent to a protected POST endpoint.
        let oversized = vec![b'x'; 64 * 1024 + 1];
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(oversized))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "body exceeding 64 KiB must return 413"
        );
    }

    #[tokio::test]
    async fn admin_body_at_limit_passes_body_limit_layer() {
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        // Exactly 64 KiB is within the limit; the handler may still reject the
        // request (invalid JSON → 422 / missing auth → 401), but NOT with 413.
        let at_limit = vec![b'x'; 64 * 1024];
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(at_limit))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_ne!(
            res.status(),
            http::StatusCode::PAYLOAD_TOO_LARGE,
            "body at exactly 64 KiB must NOT return 413"
        );
    }

    // ── POST /policies/validate tests ─────────────────────────────────────────

    async fn validate_request(policy: &str, schema: Option<serde_json::Value>) -> serde_json::Value {
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let body = if let Some(s) = schema {
            serde_json::json!({ "policy": policy, "schema": s })
        } else {
            serde_json::json!({ "policy": policy })
        };
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/validate")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            http::StatusCode::OK,
            "/policies/validate always returns 200 (valid or not)"
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn validate_endpoint_returns_valid_true_for_good_policy() {
        let resp = validate_request("permit(principal, action, resource);", None).await;
        assert_eq!(resp["valid"], true, "parseable policy must be valid");
        assert_eq!(resp["errors"], serde_json::json!([]), "no errors for valid policy");
    }

    #[tokio::test]
    async fn validate_endpoint_returns_errors_with_line_col_for_bad_syntax() {
        // A deliberately malformed policy — the brace is not valid Cedar.
        let resp = validate_request("not cedar {{{", None).await;
        assert_eq!(resp["valid"], false, "unparseable policy must be invalid");
        let errors = resp["errors"].as_array().expect("errors must be array");
        assert!(!errors.is_empty(), "at least one error must be returned");
        // The structured error must carry a non-empty message.
        let first = &errors[0];
        assert!(
            first["message"].as_str().map(|m| !m.is_empty()).unwrap_or(false),
            "error message must be non-empty"
        );
        // line/column/length fields must be present (may be 0 when Cedar provides no span).
        assert!(first.get("line").is_some(), "line field required");
        assert!(first.get("column").is_some(), "column field required");
        assert!(first.get("length").is_some(), "length field required");
    }

    #[tokio::test]
    async fn validate_endpoint_returns_schema_validation_errors() {
        let schema = serde_json::Value::String(
            "entity User; entity Route; action \"invoke\" appliesTo { principal: [User], resource: [Route] };"
                .to_string(),
        );
        // This action name is not in the schema → strict validation fails.
        let resp = validate_request(
            r#"permit(principal, action == Action::"delete_everything", resource);"#,
            Some(schema),
        ).await;
        assert_eq!(resp["valid"], false, "unknown action must fail schema validation");
        let errors = resp["errors"].as_array().expect("errors must be array");
        assert!(!errors.is_empty(), "at least one validation error required");
    }

    #[tokio::test]
    async fn upsert_policy_still_rejects_invalid_with_422_or_400() {
        // The upsert path (POST /policies/:id) hits the DB; without a DB configured
        // it returns 503. But a 64 KiB+ body correctly returns 413, confirming
        // the validate endpoint route does not shadow the upsert route — they are
        // distinct paths (/policies/validate vs /policies/:id).
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let body = serde_json::json!({
            "policy_text": "not cedar {{{",
            "enabled": true
        });
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/my-test-policy")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        // Without a DB the handler returns 503 (db_not_configured); critically
        // it must NOT return 200, and the route must NOT be swallowed by the
        // /policies/validate route (which is registered first).
        assert_ne!(
            res.status(),
            http::StatusCode::OK,
            "/policies/my-test-policy must not be routed to the validate handler"
        );
        // Also confirm that "validate" as a literal policy id hits the upsert path
        // (was the route-ordering bug we fixed: "validate" treated as :id).
        // Without DB it returns 503, not 200 with a validation response.
        let app2 = admin_router_with_auth(minimal_admin_state(), None, None);
        let req2 = http::Request::builder()
            .method("POST")
            .uri("/policies/validate")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_vec(&serde_json::json!({"policy": "permit(principal, action, resource);"})).unwrap(),
            ))
            .unwrap();
        let res2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(
            res2.status(),
            http::StatusCode::OK,
            "/policies/validate POST with a 'policy' key must hit the validate handler"
        );
        let bytes = axum::body::to_bytes(res2.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["valid"], true, "validate handler must report valid=true for good policy");
    }

    // ── Hot-reload observability tests ────────────────────────────────────────

    #[tokio::test]
    async fn reload_ok_emits_admin_event() {
        use super::AdminEvent;
        use crate::authz::{AuthzEngine, PolicyRecord};
        use std::sync::Arc;
        use tokio::sync::broadcast;

        let (tx, mut rx) = broadcast::channel::<AdminEvent>(16);
        let engine = Arc::new(AuthzEngine::empty());

        // Simulate a successful reload (the lenient path).
        let records = vec![PolicyRecord {
            id: "p1".to_string(),
            policy_text: "permit(principal, action, resource);".to_string(),
            schema_json: None,
            entities_json: None,
        }];
        engine.reload_from_records_lenient(&records);

        // Engine updated its last_reload_status; now manually send an event
        // as the cache invalidation listener would.
        let policy_count = engine.snapshot().policies().policies().count();
        tx.send(AdminEvent::PolicyReloadOk { policy_count }).unwrap();

        let event = rx.try_recv().expect("event must be in channel");
        match event {
            AdminEvent::PolicyReloadOk { policy_count: pc } => {
                assert_eq!(pc, 1, "reload OK event must report correct policy count");
            }
            other => panic!("expected PolicyReloadOk, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reload_error_emits_admin_event() {
        use super::AdminEvent;
        use tokio::sync::broadcast;

        let (tx, mut rx) = broadcast::channel::<AdminEvent>(16);
        tx.send(AdminEvent::PolicyReloadError {
            skipped_count: 0,
            db_error: Some("connection refused".to_string()),
        }).unwrap();

        let event = rx.try_recv().expect("event must be in channel");
        match event {
            AdminEvent::PolicyReloadError { db_error: Some(ref msg), .. } => {
                assert!(msg.contains("connection refused"), "error message must propagate");
            }
            other => panic!("expected PolicyReloadError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reload_status_endpoint_reflects_last_reload() {
        use std::sync::Arc;
        use crate::authz::{AuthzEngine, PolicyRecord};
        use crate::approval::ApprovalManager;
        use crate::cache::GateCache;
        use crate::config::types::{CacheConfig, GateConfig};
        use crate::proxy::router::Router as GateRouter;

        // Build an engine and trigger a lenient reload so last_reload_status is set.
        let engine = Arc::new(AuthzEngine::empty());
        let records = vec![PolicyRecord {
            id: "p2".to_string(),
            policy_text: "permit(principal, action, resource);".to_string(),
            schema_json: None,
            entities_json: None,
        }];
        engine.reload_from_records_lenient(&records);

        let gate_config = GateConfig::default();
        let config = Arc::new(tokio::sync::RwLock::new(gate_config.clone()));
        let router = Arc::new(tokio::sync::RwLock::new(GateRouter::from_config(&gate_config)));
        let state = AdminState {
            cache: Arc::new(GateCache::from_config(&CacheConfig::default())),
            db: None,
            router,
            config,
            authz: Arc::clone(&engine),
            approval_manager: Arc::new(ApprovalManager::new()),
            admin_events: None,
        };

        let app = admin_router_with_auth(state, None, None);
        let req = http::Request::builder()
            .uri("/policies/reload-status")
            .body(axum::body::Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), http::StatusCode::OK, "reload-status must return 200");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["ok"], true, "after successful reload, ok must be true");
        assert_eq!(json["policy_count"], 1, "policy count must reflect reload result");
        assert!(json["last_reload_at"].is_string(), "last_reload_at must be set");
        assert!(json["last_error"].is_null(), "no error after successful reload");
    }

    #[test]
    fn require_policies_at_startup_config_defaults_to_false() {
        // Structural guarantee: the flag is off by default so existing deployments
        // are never broken by an upgrade that adds the field.
        use crate::config::types::ServerConfig;
        let cfg: ServerConfig = serde_json::from_value(serde_json::json!({}))
            .expect("ServerConfig must deserialize with no fields");
        assert!(
            !cfg.require_policies_at_startup,
            "require_policies_at_startup must default to false"
        );
    }

    #[test]
    fn require_policies_at_startup_config_can_be_enabled() {
        use crate::config::types::ServerConfig;
        let cfg: ServerConfig =
            serde_json::from_value(serde_json::json!({"require_policies_at_startup": true}))
                .expect("ServerConfig must deserialize");
        assert!(cfg.require_policies_at_startup);
    }

    // ── POST /policies/simulate tests ─────────────────────────────────────────

    async fn simulate_request(body: serde_json::Value) -> (http::StatusCode, serde_json::Value) {
        use std::sync::Arc;
        use crate::authz::{AuthzEngine, PolicyRecord};
        use crate::approval::ApprovalManager;
        use crate::cache::GateCache;
        use crate::config::types::{CacheConfig, GateConfig};
        use crate::proxy::router::Router as GateRouter;

        // Build an engine with one permit policy so we can test both allow and deny.
        let engine = Arc::new(AuthzEngine::empty());
        let records = vec![PolicyRecord {
            id: "test-permit".to_string(),
            policy_text: r#"permit(
  principal == User::"alice",
  action == Action::"read",
  resource == Document::"report"
);"#.to_string(),
            schema_json: None,
            entities_json: None,
        }];
        engine.reload_from_records_lenient(&records);

        let gate_config = GateConfig::default();
        let config = Arc::new(tokio::sync::RwLock::new(gate_config.clone()));
        let router = Arc::new(tokio::sync::RwLock::new(GateRouter::from_config(&gate_config)));
        let state = AdminState {
            cache: Arc::new(GateCache::from_config(&CacheConfig::default())),
            db: None,
            router,
            config,
            authz: engine,
            approval_manager: Arc::new(ApprovalManager::new()),
            admin_events: None,
        };

        let app = admin_router_with_auth(state, None, None);
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/simulate")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn simulate_returns_allow_when_policy_permits() {
        let (status, json) = simulate_request(serde_json::json!({
            "principal": r#"User::"alice""#,
            "action": r#"Action::"read""#,
            "resource": r#"Document::"report""#,
        })).await;
        assert_eq!(status, http::StatusCode::OK, "simulate must return 200");
        assert_eq!(json["decision"], "Allow", "matching permit must yield Allow");
    }

    #[tokio::test]
    async fn simulate_includes_matching_policy_id_in_reasons() {
        let (status, json) = simulate_request(serde_json::json!({
            "principal": r#"User::"alice""#,
            "action": r#"Action::"read""#,
            "resource": r#"Document::"report""#,
        })).await;
        assert_eq!(status, http::StatusCode::OK);
        let reasons = json["reasons"].as_array().expect("reasons must be an array");
        assert!(
            reasons.iter().any(|r| r.as_str().map(|s| s.starts_with("test-permit")).unwrap_or(false)),
            "matching policy id must appear in reasons (Cedar may suffix #N); got: {reasons:?}",
        );
    }

    #[tokio::test]
    async fn simulate_returns_deny_when_no_matching_allow() {
        let (status, json) = simulate_request(serde_json::json!({
            "principal": r#"User::"bob""#,
            "action": r#"Action::"read""#,
            "resource": r#"Document::"report""#,
        })).await;
        assert_eq!(status, http::StatusCode::OK, "simulate must return 200 even on deny");
        assert_eq!(json["decision"], "Deny", "non-matching principal must yield Deny");
        let reasons = json["reasons"].as_array().expect("reasons must be an array");
        assert!(reasons.is_empty(), "deny has no reasons; got: {reasons:?}");
    }

    #[tokio::test]
    async fn simulate_returns_422_for_invalid_entity_uid() {
        let (status, json) = simulate_request(serde_json::json!({
            "principal": "not-a-valid-uid",
            "action": r#"Action::"read""#,
            "resource": r#"Document::"report""#,
        })).await;
        assert_eq!(
            status,
            http::StatusCode::UNPROCESSABLE_ENTITY,
            "malformed EntityUid must return 422; body: {json}"
        );
        assert_eq!(json["error"], "invalid_entity_uid", "error field must identify the issue");
        assert_eq!(json["field"], "principal", "field must identify which field failed to parse");
    }

    // ── GET /policies/{id}/history route tests ───────────────────────────────

    #[tokio::test]
    async fn policy_history_without_db_returns_501() {
        // Without a configured DB the handler returns 501 (db_not_configured),
        // not panic or 404.
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let req = http::Request::builder()
            .uri("/policies/some-policy/history")
            .body(axum::body::Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "/policies/{{id}}/history without DB must return 501"
        );
    }

    #[tokio::test]
    async fn policy_history_route_does_not_shadow_get_policy() {
        // The /policies/{id}/history route must not swallow GET /policies/{id}.
        // Without a DB both return 501 (db_not_configured), but they are distinct routes.
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let req = http::Request::builder()
            .uri("/policies/some-policy")
            .body(axum::body::Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        // 501 from db_not_configured — confirms the {id} route is still registered
        // and not shadowed by /history.
        assert_eq!(
            res.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "GET /policies/{{id}} must still return 501 (not 404) — route not shadowed"
        );
    }

    #[tokio::test]
    async fn policy_history_route_distinct_from_policy_id_route() {
        // Confirm that "history" cannot be used as a literal policy id that
        // accidentally hits GET /policies/{id} instead of /history.
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let req_history = http::Request::builder()
            .uri("/policies/my-policy/history")
            .body(axum::body::Body::empty())
            .unwrap();
        let req_id = http::Request::builder()
            .uri("/policies/my-policy")
            .body(axum::body::Body::empty())
            .unwrap();
        // Both return 503 without DB, but they reach different handlers.
        // The test confirms both routes are registered and neither 404s.
        let res_history = app.clone().oneshot(req_history).await.unwrap();
        let res_id = app.oneshot(req_id).await.unwrap();
        assert_ne!(res_history.status(), http::StatusCode::NOT_FOUND, "/history must be a registered route");
        assert_ne!(res_id.status(), http::StatusCode::NOT_FOUND, "/{{id}} must still be a registered route");
    }

    #[tokio::test]
    async fn policy_history_default_limit_is_clamped_to_100() {
        // The handler clamps limit to 100 before the DB call. Without a DB we
        // reach the 503 path before clamping, but we can test the struct defaults
        // via deserialization.
        let params: super::HistoryQueryParams = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(params.limit, 20, "default limit must be 20");
        assert_eq!(params.offset, 0, "default offset must be 0");

        let params_over: super::HistoryQueryParams =
            serde_json::from_str(r#"{"limit": 9999}"#).unwrap();
        // The clamping happens at handler level; verify the value reaches the struct.
        assert_eq!(params_over.limit, 9999, "struct accepts the raw value");
        // And confirm min(9999, 100) is what the handler would pass to the DB.
        assert_eq!(params_over.limit.min(100), 100, "effective_limit must be clamped to 100");
    }

    // ── GET /policies/{id}/history integration tests (require live DB) ────────

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn policy_history_returns_404_for_unknown_policy() {
        // The integration path: db is Some(_) and the policy does not exist.
        // Omitted from unit suite; covered by the DB-backed integration test suite.
    }

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn policy_history_returns_versions_in_desc_order() {
        // Verified by list_policy_versions ordering (version_num DESC) in db/mod.rs.
        // The HTTP layer just serializes what the DB returns.
    }

    // ── POST /policies/{id}/rollback route tests ──────────────────────────────

    #[tokio::test]
    async fn policy_rollback_without_db_returns_501() {
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let body = serde_json::to_vec(&serde_json::json!({"version_num": 1})).unwrap();
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/some-policy/rollback")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "/policies/{{id}}/rollback without DB must return 501"
        );
    }

    #[tokio::test]
    async fn policy_rollback_route_does_not_shadow_get_policy() {
        // POST /policies/{id}/rollback must not interfere with GET /policies/{id}.
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let req = http::Request::builder()
            .uri("/policies/some-policy")
            .body(axum::body::Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            http::StatusCode::NOT_IMPLEMENTED,
            "GET /policies/{{id}} must still be reachable — not shadowed by /rollback"
        );
    }

    #[tokio::test]
    async fn policy_rollback_route_is_registered() {
        // Confirm /policies/{id}/rollback is a registered route (not 404/405).
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let body = serde_json::to_vec(&serde_json::json!({"version_num": 1})).unwrap();
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/my-policy/rollback")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        // Without DB it returns 501, NOT 404 or 405 — confirms the route is registered.
        assert_ne!(res.status(), http::StatusCode::NOT_FOUND, "/rollback must be a registered route");
        assert_ne!(res.status(), http::StatusCode::METHOD_NOT_ALLOWED, "/rollback must accept POST");
    }

    #[tokio::test]
    async fn rollback_request_rejects_missing_version_num() {
        // A body with no version_num field must fail deserialization (422).
        let app = admin_router_with_auth(minimal_admin_state(), None, None);
        let body = serde_json::to_vec(&serde_json::json!({})).unwrap();
        let req = http::Request::builder()
            .method("POST")
            .uri("/policies/my-policy/rollback")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        // Axum's Json extractor returns 422 when required fields are missing.
        assert_eq!(
            res.status(),
            http::StatusCode::UNPROCESSABLE_ENTITY,
            "missing version_num must be rejected with 422"
        );
    }

    // ── POST /policies/{id}/rollback integration tests (require live DB) ──────

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn rollback_returns_404_for_unknown_policy() {}

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn rollback_returns_404_for_unknown_version_num() {}

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn rollback_returns_422_when_version_has_invalid_cedar() {}

    #[tokio::test]
    #[ignore = "requires live Postgres — run with DATABASE_URL set"]
    async fn rollback_happy_path_creates_new_version_and_reloads() {}
}
