/// Core proxy pipeline handler.
///
/// Implements the full request lifecycle:
/// 1. Route matching
/// 2. Authentication
/// 3. Template context construction
/// 4. Lookup pre-resolution (async, before sync template rendering)
/// 5. Pre-request hook execution
/// 6. Upstream proxying (streaming or buffered)
/// 7. Response forwarding + post-response usage logging
use crate::auth::{AuthError, AuthMethod, Authenticator, Identity, SharedJwtMinter};
use crate::cache::GateCache;
use crate::config::{
    lookup::collect_hook_templates,
    types::{BudgetWindow, GateConfig, MaxTokenBudgetConfig, PostResponseHook, PreRequestHook},
    LookupRegistry, TemplateContext, TemplateEngine,
};
use crate::db::{AuthzAuditDecision, AuthzAuditRecord, Database, UsageEvent};
use crate::guardrail::{build_guardrail, GuardrailInput, GuardrailOutcome};
use crate::proxy::SharedRouter;
use crate::stream::{NdjsonStreamProcessor, SseStreamProcessor, StreamProcessor};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Headers that must not be forwarded to the upstream (hop-by-hop).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Application state shared across all requests.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub config: Arc<RwLock<GateConfig>>,
    pub router: SharedRouter,
    pub auth_providers: Arc<HashMap<String, Arc<dyn Authenticator>>>,
    pub jwt_minter: SharedJwtMinter,
    pub cache: Arc<GateCache>,
    pub db: Option<Arc<Database>>,
    pub http_client: reqwest::Client,
    pub lookup_registry: Arc<LookupRegistry>,
    /// Embedded Cedar authorization engine, shared lock-free via `ArcSwap`.
    /// The `Authorize` pre-request hook evaluates route-level decisions against
    /// the live policy bundle held here.
    pub authz: Arc<crate::authz::AuthzEngine>,
    /// Shared human-in-the-loop approval routing table. Each stream task
    /// registers its pending approvals here; the Admin API resolves them.
    pub approval_manager: crate::approval::ApprovalManager,
    /// Shared Redis-backed window counters for authoritative token budgets and
    /// request-rate limits. `None` when the `redis-l2` feature is disabled or
    /// Redis is not configured — callers then use the Postgres windowed sum.
    #[cfg(feature = "redis-l2")]
    pub rate_limiter: Option<crate::ratelimit::RedisRateLimiter>,
}

/// The main proxy handler — catches all requests on the proxy port.
pub async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let span = tracing::info_span!(
        "proxy_request",
        request_id = %request_id,
        method = %req.method(),
        uri = %req.uri(),
    );
    let _enter = span.enter();

    match handle_request(state, req, &request_id).await {
        Ok(response) => response,
        Err(status) => {
            warn!(request_id = %request_id, status = %status, "request failed");
            status.into_response()
        }
    }
}

async fn handle_request(
    state: Arc<AppState>,
    req: axum::extract::Request,
    request_id: &str,
) -> Result<Response, StatusCode> {
    // ── 1. Extract request parts ───────────────────────────────────────────
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    // Determine host from Host header
    let host = headers
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();

    let path = uri.path();
    let method_str = method.as_str();

    // ── 2. Route matching ──────────────────────────────────────────────────
    let router = state.router.read().await;
    let matched_route = match router.match_route(&host, path, method_str) {
        Some(r) => r.clone(),
        None => {
            info!(host = %host, path = %path, "no route matched");
            return Err(StatusCode::NOT_FOUND);
        }
    };
    drop(router);

    let route_id = matched_route.config.id.clone();
    info!(
        request_id = %request_id,
        route_id = %route_id,
        "route matched"
    );

    // ── 3. Authentication ──────────────────────────────────────────────────
    let auth_provider_name = matched_route
        .config
        .auth
        .as_deref()
        .or(matched_route.site.default_auth.as_deref());

    // Is the resolved provider an MCP Resource Server? If so, auth failures emit
    // OAuth 2.1 `WWW-Authenticate` discovery/step-up headers, and the inbound
    // access token is stripped before proxying (confused-deputy guard).
    let mcp_provider_cfg: Option<crate::config::types::McpAuthConfig> =
        if let Some(name) = auth_provider_name {
            match state.config.read().await.auth_providers.get(name) {
                Some(crate::config::types::AuthProviderConfig::Mcp(cfg)) => Some(cfg.clone()),
                _ => None,
            }
        } else {
            None
        };
    let is_mcp_auth = mcp_provider_cfg.is_some();

    // Extract the raw credential for cache key derivation (Authorization or Cookie).
    let raw_credential = parts
        .headers
        .get(http::header::AUTHORIZATION)
        .or_else(|| parts.headers.get(http::header::COOKIE))
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let auth_result = if let Some(provider_name) = auth_provider_name {
        match state.auth_providers.get(provider_name) {
            Some(auth) => {
                // Fast path: check session cache before hitting the upstream auth provider.
                // Only cache Kratos-style session results; JWT and API key authenticators
                // manage their own caching internally.
                let cached = if let Some(ref cred) = raw_credential {
                    state.cache.get_session(cred).await
                } else {
                    None
                };

                if let Some(cached_identity) = cached {
                    info!(request_id = %request_id, user_id = %cached_identity.id, "session cache hit");
                    crate::auth::AuthResult {
                        identity: cached_identity,
                        method: AuthMethod::KratosSession,
                    }
                } else {
                    match auth.authenticate(&parts).await {
                        Ok(result) => {
                            // Populate session cache for Kratos results.
                            if matches!(result.method, AuthMethod::KratosSession) {
                                if let Some(ref cred) = raw_credential {
                                    state.cache.put_session(cred, &result.identity).await;
                                }
                            }
                            result
                        }
                        Err(AuthError::Unauthorized(msg)) => {
                            warn!(request_id = %request_id, provider = %provider_name, reason = %msg, "authentication failed");
                            // MCP RS: 401 carries a discovery pointer to the
                            // Protected Resource Metadata (RFC 9728 + OAuth 2.1).
                            if is_mcp_auth {
                                return Ok(mcp_discovery_unauthorized(&host, &parts));
                            }
                            return Err(StatusCode::UNAUTHORIZED);
                        }
                        Err(AuthError::InsufficientScope { required }) => {
                            warn!(request_id = %request_id, provider = %provider_name, ?required, "insufficient scope");
                            // Audit the step-up (403 insufficient_scope). The
                            // principal is not yet authenticated at this seam, so
                            // it is recorded as "anonymous"; the required scopes
                            // are carried in the context for the trail.
                            record_authz_decision(
                                &state.db,
                                AuthzAuditRecord {
                                    request_id: Some(request_id.to_string()),
                                    principal: "anonymous".to_string(),
                                    action: method_str.to_string(),
                                    resource: route_id.clone(),
                                    decision: AuthzAuditDecision::StepUp,
                                    reason: Some(
                                        "insufficient_scope — step-up required".to_string(),
                                    ),
                                    context: Some(serde_json::json!({
                                        "required_scopes": required,
                                        "provider": provider_name,
                                        "path": path,
                                    })),
                                },
                            );
                            // 403 step-up: tell the client which scope to request.
                            return Ok(mcp_insufficient_scope(&required));
                        }
                        Err(AuthError::ProviderError(msg)) => {
                            error!(request_id = %request_id, provider = %provider_name, error = %msg, "auth provider error");
                            return Err(StatusCode::BAD_GATEWAY);
                        }
                        Err(AuthError::NotConfigured) => crate::auth::AuthResult {
                            identity: Identity::anonymous("anonymous"),
                            method: AuthMethod::Anonymous,
                        },
                    }
                }
            }
            None => {
                error!(request_id = %request_id, provider = %provider_name, "configured auth provider not found");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    } else {
        crate::auth::AuthResult {
            identity: Identity::anonymous("anonymous"),
            method: AuthMethod::Anonymous,
        }
    };

    let identity = auth_result.identity;
    info!(
        request_id = %request_id,
        user_id = %identity.id,
        "authenticated"
    );

    // ── 4. Read request body ───────────────────────────────────────────────
    const MAX_BODY_SIZE: usize = 32 * 1024 * 1024; // 32 MiB
    let body_bytes = axum::body::to_bytes(body, MAX_BODY_SIZE)
        .await
        .map_err(|e| {
            warn!(request_id = %request_id, error = %e, "failed to read request body");
            StatusCode::BAD_REQUEST
        })?;

    // ── 5. Build template context ──────────────────────────────────────────
    let body_value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
    };

    let mut api_key_ctx = HashMap::new();
    if let AuthMethod::ApiKey { client_id, scopes } = &auth_result.method {
        api_key_ctx.insert("client_id".to_string(), client_id.clone());
        api_key_ctx.insert("scopes".to_string(), scopes.join(","));
    }

    let mut template_ctx = TemplateContext::new(
        identity.to_value(),
        body_value.clone(),
        request_id.to_string(),
        api_key_ctx,
    );

    // ── 5b. Pre-resolve async lookups ─────────────────────────────────────
    {
        let hook_templates = collect_hook_templates(&matched_route.config.hooks.pre_request);
        let template_refs: Vec<&str> = hook_templates.iter().map(String::as_str).collect();
        let resolved = state
            .lookup_registry
            .resolve_all(&template_refs, &template_ctx)
            .await;
        template_ctx.lookups = resolved;
    }

    // ── 6. Pre-request hooks ───────────────────────────────────────────────
    let mut injected_headers: HashMap<String, String> = HashMap::new();
    let mut body_overrides: HashMap<String, String> = HashMap::new();
    let mut minted_jwt: Option<String> = None;

    for hook in &matched_route.config.hooks.pre_request {
        match hook {
            PreRequestHook::ClaimsEnhancement { config } => {
                // Inject headers via template
                for (header_name, template) in &config.inject_headers {
                    let value = TemplateEngine::render(template, &template_ctx);
                    injected_headers.insert(header_name.clone(), value);
                }
                // Optionally mint a JWT
                if let Some(mint_cfg) = &config.mint_jwt {
                    if mint_cfg.enabled {
                        let minter_guard = state.jwt_minter.read().await;
                        if let Some(minter) = minter_guard.as_ref() {
                            match minter.mint(&identity, Some(&mint_cfg.additional_claims), None) {
                                Ok(token) => {
                                    minted_jwt = Some(token);
                                }
                                Err(e) => {
                                    warn!(request_id = %request_id, error = %e, "JWT minting failed");
                                }
                            }
                        }
                    }
                }
            }
            PreRequestHook::BodyTransform { config } => {
                for (field, template) in &config.set_fields {
                    let value = TemplateEngine::render(template, &template_ctx);
                    body_overrides.insert(field.clone(), value);
                }
            }
            PreRequestHook::MaxTokenBudget { config } => {
                let user_id = TemplateEngine::render(
                    &format!("{{{{ {} }}}}", config.user_id_expr),
                    &template_ctx,
                );
                // `Lifetime` reads the value the lookup registry pre-resolved.
                // Windowed budgets are resolved inline: the pre-resolve lookup
                // pattern is read-only and keyed by string, so it cannot carry
                // the token amount the metering step must INCR, nor branch on
                // Redis-vs-Postgres. This arm is already async, so inline async
                // resolution is both correct and simpler than a second lookup.
                let usage = resolve_budget_usage(&state, config, &user_id, &template_ctx).await;
                let used = match usage {
                    BudgetUsage::Known(u) => u,
                    BudgetUsage::Unavailable => {
                        // Outage posture: an Agent budget is a hard governance
                        // control, so a backend outage fails CLOSED (deny); a
                        // User/Team budget degrades (allow + WARN) to avoid
                        // hard-blocking human traffic on a transient blip.
                        //
                        // Defense in depth: the deny also fires when the ACTUAL
                        // principal is an Agent (`derived_kind`), even if the route
                        // left the budget at a non-agent scope — so a delegated
                        // agent never silently escapes fail-closed via a mis-scoped
                        // (`scope: user`) budget.
                        if outage_must_deny(config.scope, &identity) {
                            warn!(
                                request_id = %request_id,
                                user_id = %user_id,
                                scope = config.scope.tag(),
                                window = config.window.tag(),
                                "budget backend unavailable — DENYING (fail-closed, agent scope)"
                            );
                            crate::metrics::record_agent_budget_denied();
                            return Ok(quota_exceeded_response(&budget_error_message(config)));
                        }
                        warn!(
                            request_id = %request_id,
                            user_id = %user_id,
                            scope = config.scope.tag(),
                            "budget backend unavailable — degrading (allow) for non-agent scope"
                        );
                        0
                    }
                };
                if budget_exceeded(used, config.limit) {
                    let msg = budget_error_message(config);
                    warn!(
                        request_id = %request_id,
                        user_id = %user_id,
                        used,
                        limit = config.limit,
                        window = config.window.tag(),
                        "token budget exceeded — blocking request"
                    );
                    // Count over-budget blocks of agent principals/scopes so the
                    // volume of enforced agent spend caps is observable.
                    if outage_must_deny(config.scope, &identity) {
                        crate::metrics::record_agent_budget_denied();
                    }
                    return Ok(quota_exceeded_response(&msg));
                }
            }
            PreRequestHook::Authorize { config } => {
                // Build the Cedar request from identity + route + request attrs.
                // The action is generic (default "invoke"); per-route/per-tool
                // distinctions live in `context`, not in distinct action ids.
                let authz_context = build_authz_context(&identity, &route_id, method_str, path);
                // Authorize with the principal's real KIND (User/Agent/Service)
                // so a policy scoped to a non-human principal matches correctly
                // and a User-scoped policy never matches an agent with the same id.
                let principal_kind = crate::auth::identity::principal_kind_for(&identity);
                // A revoked non-human identity is denied here too (parity with the
                // per-tool gate) — fail-closed on a DB error.
                let nhi_revoked = if matches!(
                    principal_kind,
                    crate::authz::PrincipalKind::Agent | crate::authz::PrincipalKind::Service
                ) {
                    match &state.db {
                        Some(db) => db.is_agent_revoked(&identity.id).await.unwrap_or_else(|e| {
                            warn!(error = %e, principal = %identity.id, "revocation check failed — denying (fail-closed)");
                            true
                        }),
                        None => false,
                    }
                } else {
                    false
                };
                let decision = if nhi_revoked {
                    crate::authz::AuthzDecision::Deny
                } else {
                    state.authz.authorize_as(
                        principal_kind,
                        &identity.id,
                        &config.action,
                        &route_id,
                        &authz_context,
                    )
                };
                if !decision.is_allow() {
                    if config.enforce {
                        warn!(
                            request_id = %request_id,
                            user_id = %identity.id,
                            route_id = %route_id,
                            action = %config.action,
                            "authorization denied — blocking request (403)"
                        );
                        // Audit the enforced deny (best-effort, non-blocking).
                        record_authz_decision(
                            &state.db,
                            AuthzAuditRecord {
                                request_id: Some(request_id.to_string()),
                                principal: identity.id.clone(),
                                action: config.action.clone(),
                                resource: route_id.clone(),
                                decision: AuthzAuditDecision::Deny,
                                reason: Some("authorization denied (enforce=true)".to_string()),
                                context: Some(authz_context.clone()),
                            },
                        );
                        let msg = config
                            .error_message
                            .clone()
                            .unwrap_or_else(|| "authorization denied".to_string());
                        return Ok(forbidden_response(&msg));
                    }
                    // Shadow / audit mode: log the deny but let the request pass.
                    warn!(
                        request_id = %request_id,
                        user_id = %identity.id,
                        route_id = %route_id,
                        action = %config.action,
                        "authorization would deny (enforce=false) — allowing request"
                    );
                    // Record the shadow-mode deny so audit reflects what a policy
                    // WOULD have blocked; the request itself still proceeds.
                    record_authz_decision(
                        &state.db,
                        AuthzAuditRecord {
                            request_id: Some(request_id.to_string()),
                            principal: identity.id.clone(),
                            action: config.action.clone(),
                            resource: route_id.clone(),
                            decision: AuthzAuditDecision::Deny,
                            reason: Some(
                                "authorization would deny (enforce=false, shadow)".to_string(),
                            ),
                            context: Some(authz_context.clone()),
                        },
                    );
                } else {
                    // Record the allow so the audit trail carries the full
                    // route-level decision stream, not only denials.
                    record_authz_decision(
                        &state.db,
                        AuthzAuditRecord {
                            request_id: Some(request_id.to_string()),
                            principal: identity.id.clone(),
                            action: config.action.clone(),
                            resource: route_id.clone(),
                            decision: AuthzAuditDecision::Allow,
                            reason: None,
                            context: Some(authz_context.clone()),
                        },
                    );
                }
            }
            PreRequestHook::Guardrail { config } => {
                let guard = build_guardrail(&config.guard);
                let input = GuardrailInput {
                    request_id: request_id.to_string(),
                    route_id: route_id.clone(),
                    principal_id: identity.id.clone(),
                    method: method_str.to_string(),
                    path: path.to_string(),
                    headers: headers
                        .iter()
                        .filter_map(|(k, v)| {
                            v.to_str()
                                .ok()
                                .map(|s| (k.as_str().to_string(), s.to_string()))
                        })
                        .collect(),
                    body: body_value.clone(),
                };
                let outcome = guard.inspect(&input).await;
                match outcome {
                    GuardrailOutcome::Allow => {}
                    GuardrailOutcome::Block { reason } => {
                        if config.enforce {
                            warn!(
                                request_id = %request_id,
                                user_id = %identity.id,
                                route_id = %route_id,
                                "guardrail blocked request"
                            );
                            let msg = config.error_message.clone().unwrap_or(reason);
                            return Ok(guardrail_blocked_response(&msg));
                        }
                        warn!(
                            request_id = %request_id,
                            user_id = %identity.id,
                            route_id = %route_id,
                            "guardrail would block (enforce=false) — allowing request"
                        );
                    }
                    GuardrailOutcome::Annotate { labels } => {
                        info!(
                            request_id = %request_id,
                            user_id = %identity.id,
                            route_id = %route_id,
                            labels = ?labels,
                            "guardrail annotation"
                        );
                    }
                }
            }
        }
    }

    // Apply minted JWT as Authorization header
    if let Some(jwt) = minted_jwt {
        injected_headers.insert("Authorization".to_string(), format!("Bearer {jwt}"));
    }

    // Apply body transforms
    let final_body_bytes = if body_overrides.is_empty() {
        body_bytes
    } else {
        apply_body_transforms(&body_bytes, &body_overrides)
    };

    // ── 7. Build upstream request ──────────────────────────────────────────
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or(path);

    let upstream_url = match crate::proxy::router::Router::resolve_upstream(
        &matched_route,
        path_and_query,
    ) {
        Some(url) => url,
        None => {
            error!(request_id = %request_id, route_id = %route_id, "no upstream URL configured");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    info!(request_id = %request_id, upstream = %upstream_url, "proxying to upstream");

    let mut upstream_req = state.http_client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        &upstream_url,
    );

    // Forward non-hop-by-hop headers
    let mut fwd_headers = reqwest::header::HeaderMap::new();
    for (name, value) in &headers {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        // Confused-deputy guard (RFC 8707 §3, MCP auth spec): the inbound MCP
        // access token was minted for THIS resource server. It MUST NOT be
        // forwarded upstream, where a compromised/rogue upstream could replay
        // it against the AS or another RS. Drop the Authorization header on the
        // MCP-auth path; a downstream-facing credential (if any) is re-attached
        // only via an explicit ClaimsEnhancement mint_jwt hook below.
        if is_mcp_auth && name_str == "authorization" {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_str(name.as_str()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            fwd_headers.insert(n, v);
        }
    }

    // Inject hook-generated headers
    for (name, value) in &injected_headers {
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_str(name),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            fwd_headers.insert(n, v);
        }
    }

    // Set X-Request-Id
    if let Ok(v) = reqwest::header::HeaderValue::from_str(request_id) {
        fwd_headers.insert("x-request-id", v);
    }

    upstream_req = upstream_req.headers(fwd_headers);

    // Attach body
    if !final_body_bytes.is_empty() {
        upstream_req = upstream_req.body(final_body_bytes.to_vec());
    }

    // ── 8. Send upstream request ───────────────────────────────────────────
    let upstream_response = upstream_req.send().await.map_err(|e| {
        error!(request_id = %request_id, error = %e, upstream = %upstream_url, "upstream request failed");
        StatusCode::BAD_GATEWAY
    })?;

    let status = upstream_response.status();
    let resp_headers = upstream_response.headers().clone();

    // ── 9. Build response ──────────────────────────────────────────────────
    let mut response_builder = Response::builder().status(status.as_u16());

    // Forward response headers
    for (name, value) in &resp_headers {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_str(name.as_str()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            response_builder = response_builder.header(n, v);
        }
    }

    // Always forward X-Request-Id
    response_builder = response_builder.header("x-request-id", request_id);

    // ── 10. Stream or buffer response ─────────────────────────────────────
    let stream_enabled = matched_route.config.stream.enabled;
    let request_start = std::time::Instant::now();

    let response_body = if stream_enabled {
        let stream_config = matched_route.config.stream.clone();

        // Extract user scopes for A2UI filtering
        let user_scopes: Vec<String> = identity
            .extra
            .get("a2ui_scopes")
            .map(|s| s.split(',').map(|sc| sc.trim().to_string()).collect())
            .unwrap_or_default();

        // Render AG-UI inject_metadata templates against the per-request context
        let ag_ui_metadata = {
            let mut map = serde_json::Map::new();
            for (key, template) in &stream_config.ai.ag_ui.inject_metadata {
                let rendered = TemplateEngine::render(template, &template_ctx);
                let value = serde_json::from_str::<serde_json::Value>(&rendered)
                    .unwrap_or(serde_json::Value::String(rendered));
                map.insert(key.clone(), value);
            }
            map
        };

        // A2UI theme from config
        let a2ui_theme = stream_config.ai.a2ui.theme.clone();

        // Per-tool-call authorization: enabled for a route that carries an
        // enforcing `Authorize` hook. That hook already opted the route into
        // Cedar authz; per-tool authz extends the same policy bundle to each
        // in-stream tool call and to `list_tools` visibility. Routes without an
        // enforcing Authorize hook get `None` — completely unaffected.
        let tool_authz_ctx =
            build_tool_authz_context(&state, &matched_route, &identity, &route_id, request_id).await;

        // Human-in-the-loop approvals: each stream gets a private notification
        // channel. The shared ApprovalManager routes Admin API decisions back
        // to this task using the channel; the task then asks the processor to
        // release the buffered call.
        let (approval_tx, mut approval_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, crate::approval::ApprovalDecision)>();
        let approval_handle = Some((state.approval_manager.clone(), approval_tx));

        // Protocol dispatch: create the appropriate processor
        let processor: Box<dyn StreamProcessor> = match stream_config.protocol.as_str() {
            "ndjson" => Box::new(NdjsonStreamProcessor::with_tool_authz_and_approval(
                stream_config.clone(),
                user_scopes.clone(),
                ag_ui_metadata.clone(),
                a2ui_theme.clone(),
                tool_authz_ctx.clone(),
                approval_handle.clone(),
            )),
            _ => Box::new(SseStreamProcessor::with_tool_authz_and_approval(
                stream_config.clone(),
                user_scopes.clone(),
                ag_ui_metadata.clone(),
                a2ui_theme.clone(),
                tool_authz_ctx.clone(),
                approval_handle.clone(),
            )),
        };

        // Session watchdog: spawn a periodic re-validation task when enabled
        let watchdog_cancel = tokio_util::sync::CancellationToken::new();
        if let Some(ref sw) = stream_config.ai.session_watchdog {
            if sw.enabled {
                let interval_secs = sw.check_interval_seconds;
                let credential = raw_credential.clone();
                let cache = state.cache.clone();
                let cancel = watchdog_cancel.clone();

                tokio::spawn(async move {
                    let mut ticker =
                        tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                    ticker.tick().await; // skip immediate first tick
                    loop {
                        tokio::select! {
                            _ = ticker.tick() => {
                                if let Some(ref cred) = credential {
                                    match cache.get_session(cred).await {
                                        Some(_) => { /* session still cached — valid */ }
                                        None => {
                                            tracing::warn!(
                                                "session watchdog: session no longer valid — terminating stream"
                                            );
                                            cancel.cancel();
                                            break;
                                        }
                                    }
                                }
                            }
                            _ = cancel.cancelled() => break,
                        }
                    }
                });
            }
        }

        let mut processor = processor;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
        // oneshot channel: streaming task sends total tokens when stream is done
        let (metrics_tx, metrics_rx) = tokio::sync::oneshot::channel::<u64>();

        let stream_cancel = watchdog_cancel.clone();
        let term_payload = processor.termination_payload();

        tokio::spawn(async move {
            let mut byte_stream = upstream_response.bytes_stream();
            loop {
                // When approvals are pending we pause upstream reads and wait
                // for decisions. This keeps the buffered tool call from
                // receiving further events until the human/frontend resolves it.
                if processor.pending_approvals().is_empty() {
                    tokio::select! {
                        biased;
                        _ = stream_cancel.cancelled() => {
                            let _ = tx.send(Ok(Bytes::from(term_payload.clone()))).await;
                            break;
                        }
                        chunk = byte_stream.next() => {
                            match chunk {
                                Some(Ok(bytes)) => {
                                    match processor.process_chunk(&bytes) {
                                        Some(processed) if !processed.is_empty() => {
                                            if tx.send(Ok(processed)).await.is_err() {
                                                break;
                                            }
                                        }
                                        None => {
                                            let _ = tx.send(Ok(Bytes::from(term_payload.clone()))).await;
                                            break;
                                        }
                                        Some(_) => {}
                                    }
                                }
                                Some(Err(e)) => {
                                    let _ = tx.send(Err(std::io::Error::other(e))).await;
                                    break;
                                }
                                None => break,
                            }
                        }
                    }
                } else {
                    tokio::select! {
                        biased;
                        _ = stream_cancel.cancelled() => {
                            let _ = tx.send(Ok(Bytes::from(term_payload.clone()))).await;
                            break;
                        }
                        decision = approval_rx.recv() => {
                            match decision {
                                Some((approval_id, dec)) => {
                                    if let Some(bytes) = processor.resolve_approval(&approval_id, dec) {
                                        if !bytes.is_empty() && tx.send(Ok(bytes)).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
            // Send total token count to the post-response task
            let _ = metrics_tx.send(processor.metrics().estimated_tokens);
        });

        // Post-response: log usage only when a StreamMeter hook with log_to_db=true is configured.
        let should_log = matched_route
            .config
            .hooks
            .post_response
            .iter()
            .any(|h| matches!(h, PostResponseHook::StreamMeter { config } if config.log_to_db));
        // Collect windowed budget hooks so the metering step can advance the
        // shared Redis counters by the actual token count once the stream ends.
        let windowed_budgets =
            collect_windowed_budgets(&matched_route.config.hooks.pre_request, &template_ctx);

        if should_log || !windowed_budgets.is_empty() {
            let db_opt = state.db.clone();
            #[cfg(feature = "redis-l2")]
            let limiter = state.rate_limiter.clone();
            let user_id = identity.id.clone();
            let rid = request_id.to_string();
            let rid_clone = rid.clone();
            tokio::spawn(async move {
                let tokens = metrics_rx.await.unwrap_or(0);
                let duration_ms = request_start.elapsed().as_millis() as u64;

                // Advance shared Redis window counters by the tokens consumed.
                #[cfg(feature = "redis-l2")]
                {
                    if let Some(ref limiter) = limiter {
                        for (scope, window, budget_id) in &windowed_budgets {
                            if let Err(e) = limiter
                                .incr_budget(*scope, budget_id, *window, tokens)
                                .await
                            {
                                tracing::warn!(error = %e, request_id = %rid, "failed to advance windowed budget counter");
                            }
                        }
                    }
                }
                #[cfg(not(feature = "redis-l2"))]
                let _ = &windowed_budgets;

                // Durable ledger write (unchanged) — only when a StreamMeter
                // hook requested it.
                if should_log {
                    if let Some(db) = db_opt {
                        let event =
                            UsageEvent::new(rid_clone, user_id, route_id, tokens, duration_ms);
                        if let Err(e) = db.log_usage(&event).await {
                            tracing::warn!(error = %e, request_id = %rid, "failed to log usage event");
                        }
                    }
                }
            });
        } else {
            drop(metrics_rx);
        }

        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx))
    } else {
        let bytes = upstream_response.bytes().await.map_err(|e| {
            error!(request_id = %request_id, error = %e, "failed to read upstream response body");
            StatusCode::BAD_GATEWAY
        })?;

        // Task 3: filter denied tools out of a buffered MCP `tools/list`
        // response. MCP tool listings most commonly arrive as a single
        // JSON-RPC body (not SSE). Only attempt this when the route opts into
        // per-tool authz (an enforcing Authorize hook); otherwise forward
        // untouched. `filter_list_tools_body` returns `None` for any body that
        // is not a tools/list result, so non-listing responses are unaffected.
        let bytes = if let Some(ctx) =
            build_tool_authz_context(&state, &matched_route, &identity, &route_id, request_id).await
        {
            match crate::authz::filter_list_tools_body(
                &bytes,
                &ctx.engine,
                &ctx.principal_id,
                &ctx.route_id,
            ) {
                Some(filtered) => Bytes::from(filtered),
                None => bytes,
            }
        } else {
            bytes
        };

        Body::from(bytes)
    };

    response_builder.body(response_body).map_err(|e| {
        error!(error = %e, "failed to build response");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Collect the `(scope, window, resolved_id)` triples for every windowed
/// `MaxTokenBudget` hook on a route. `Lifetime` budgets are excluded — they are
/// accounted purely from the durable `usage_events` ledger. The resolved id
/// matches what the pre-request check used, so the metering INCR advances the
/// same counter the check reads.
#[allow(unused_variables)]
fn collect_windowed_budgets(
    hooks: &[PreRequestHook],
    template_ctx: &TemplateContext,
) -> Vec<(crate::config::types::BudgetScope, BudgetWindow, String)> {
    hooks
        .iter()
        .filter_map(|hook| match hook {
            PreRequestHook::MaxTokenBudget { config }
                if config.window != BudgetWindow::Lifetime =>
            {
                let id = TemplateEngine::render(
                    &format!("{{{{ {} }}}}", config.user_id_expr),
                    template_ctx,
                );
                Some((config.scope, config.window, id))
            }
            _ => None,
        })
        .collect()
}

/// Pure decision: does `used` meet or exceed `limit`? Blocking is inclusive
/// (`used >= limit`) so a budget that has been exactly consumed still blocks
/// the next request, matching the original lifetime behavior.
fn budget_exceeded(used: u64, limit: u64) -> bool {
    used >= limit
}

/// Whether a budget-backend **outage** must DENY the request for this scope.
///
/// Fail-closed for `Agent` — an agent budget is a hard governance control, so it
/// must never be silently bypassed by a Redis/DB blip. `User`/`Team` budgets
/// degrade (allow) to preserve human-traffic availability, matching the
/// pre-existing best-effort behavior. This mirrors the OAuth rate-limiter's
/// [`BackendUnavailablePosture`](crate::config::types::BackendUnavailablePosture):
/// Agent = Deny, others = Degrade.
fn budget_outage_denies(scope: crate::config::types::BudgetScope) -> bool {
    matches!(scope, crate::config::types::BudgetScope::Agent)
}

/// Whether a budget-backend outage must DENY, considering BOTH the operator-
/// declared scope AND the request's actual principal kind (defense in depth): an
/// Agent-scoped budget denies, AND an actual `Agent` principal denies even under a
/// mis-scoped (non-agent) budget, so a delegated agent can never silently escape
/// fail-closed via a route that left the budget at `scope: user`.
///
/// NOTE: fail-closed applies to **windowed** budgets (Minute/Hour/Day). A
/// `Lifetime` budget is ledger-only and keeps its best-effort (allow-on-error)
/// read — an agent budget that must fail closed requires a fixed window.
fn outage_must_deny(scope: crate::config::types::BudgetScope, identity: &Identity) -> bool {
    budget_outage_denies(scope)
        || identity.derived_kind() == crate::auth::identity::IdentityKind::Agent
}

/// The 429 error message for a budget hook — the configured override, or the
/// default `"token budget exceeded"`.
fn budget_error_message(config: &MaxTokenBudgetConfig) -> String {
    config
        .error_message
        .clone()
        .unwrap_or_else(|| "token budget exceeded".to_string())
}

/// Derive the absolute URL of this proxy's RFC 9728 Protected Resource Metadata
/// document from the inbound request. Scheme is inferred from the
/// `x-forwarded-proto` header (TLS terminators / load balancers set it),
/// defaulting to `https` — never downgrade a discovery pointer to `http`.
fn resource_metadata_url(host: &str, parts: &http::request::Parts) -> String {
    let scheme = parts
        .headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|s| *s == "http" || *s == "https")
        .unwrap_or("https");
    format!(
        "{scheme}://{host}{}",
        crate::auth::mcp_metadata::PROTECTED_RESOURCE_METADATA_PATH
    )
}

/// Build the `401 Unauthorized` response for an MCP-protected route, carrying the
/// `WWW-Authenticate: Bearer resource_metadata="…"` discovery header (RFC 9728 +
/// OAuth 2.1). The body is a small JSON envelope for humans/debuggers.
fn mcp_discovery_unauthorized(host: &str, parts: &http::request::Parts) -> Response {
    let metadata_url = resource_metadata_url(host, parts);
    let header = crate::auth::mcp_metadata::www_authenticate_discovery(&metadata_url);
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("content-type", "application/json")
        .header(http::header::WWW_AUTHENTICATE, header)
        .body(Body::from(
            serde_json::json!({"error": "unauthorized"}).to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::UNAUTHORIZED.into_response())
}

/// Build the `403 Forbidden` step-up response for an MCP-protected route whose
/// token verified but lacked a required scope. Emits
/// `WWW-Authenticate: Bearer error="insufficient_scope", scope="…"` so the
/// client knows which scope to request from the AS.
fn mcp_insufficient_scope(required: &[String]) -> Response {
    let header = crate::auth::mcp_metadata::www_authenticate_insufficient_scope(required);
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "application/json")
        .header(http::header::WWW_AUTHENTICATE, header)
        .body(Body::from(
            serde_json::json!({
                "error": "insufficient_scope",
                "scope": required.join(" "),
            })
            .to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::FORBIDDEN.into_response())
}

/// Build the `429 Too Many Requests` JSON response used when a budget is
/// exceeded: `{"error":"quota_exceeded","message":<msg>}`. Falls back to a
/// minimal body if the builder somehow fails (it never does for these inputs).
fn quota_exceeded_response(msg: &str) -> Response {
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"error": "quota_exceeded", "message": msg}).to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::TOO_MANY_REQUESTS.into_response())
}

/// Build the Cedar request `context` record for an `Authorize` hook.
///
/// Carries the request attributes a policy may branch on: HTTP method, path,
/// route id, and the authenticated principal's id. Kept as a plain JSON object
/// so [`crate::authz::AuthzEngine::authorize`] maps it into a Cedar context.
/// Identity traits are included as a nested object when present so policies can
/// reference e.g. `context.identity.email` without a schema change.
fn build_authz_context(identity: &Identity, route_id: &str, method: &str, path: &str) -> Value {
    serde_json::json!({
        "method": method,
        "path": path,
        "route_id": route_id,
        "principal_id": identity.id,
        "identity": identity.traits,
    })
}

/// Build a [`ToolAuthzContext`](crate::authz::ToolAuthzContext) for the stream
/// when the route opts into per-tool authorization.
///
/// The opt-in signal is an *enforcing* `Authorize` pre-request hook on the
/// route: that hook already authorized the connection against the Cedar bundle,
/// and per-tool authz extends the same bundle to each in-stream tool call. A
/// shadow-mode (`enforce = false`) Authorize hook does NOT enable blocking of
/// tool calls — it only logs at the route level — so it yields `None` here too,
/// keeping the enforce semantics consistent between route and tool granularity.
///
/// Returns `None` when no such hook is present, leaving the stream unaffected.
async fn build_tool_authz_context(
    state: &Arc<AppState>,
    matched_route: &crate::proxy::router::CompiledRoute,
    identity: &Identity,
    route_id: &str,
    request_id: &str,
) -> Option<crate::authz::ToolAuthzContext> {
    let enforcing = matched_route
        .config
        .hooks
        .pre_request
        .iter()
        .any(|h| matches!(h, PreRequestHook::Authorize { config } if config.enforce));
    if !enforcing {
        return None;
    }
    // Attach a best-effort audit sink when a DB is configured so per-tool
    // denials are recorded off the streaming hot path.
    let audit = state
        .db
        .as_ref()
        .map(|db| crate::authz::ToolAuditSink::new(db.clone(), request_id));

    let principal_kind = crate::auth::identity::principal_kind_for(identity);

    // For a non-human identity, consult the revocation list once per request.
    // A revoked NHI is denied on the next authorize (fail-closed). On a DB error
    // we fail CLOSED (treat as revoked) rather than let a revoked agent through.
    let revoked = if matches!(
        principal_kind,
        crate::authz::PrincipalKind::Agent | crate::authz::PrincipalKind::Service
    ) {
        match &state.db {
            Some(db) => db.is_agent_revoked(&identity.id).await.unwrap_or_else(|e| {
                warn!(error = %e, principal = %identity.id, "revocation check failed — denying (fail-closed)");
                true
            }),
            None => false,
        }
    } else {
        false
    };

    Some(crate::authz::ToolAuthzContext {
        engine: state.authz.clone(),
        principal_kind,
        principal_id: identity.id.clone(),
        route_id: route_id.to_string(),
        revoked,
        audit,
    })
}

/// Record one authorization decision to the audit trail, off the hot path.
///
/// Best-effort and NON-BLOCKING: when a DB is configured we clone the
/// `Arc<Database>` and `tokio::spawn` the insert so a slow or failing write can
/// never block or fail the request — the authorization decision that produced
/// this record already stands regardless of whether the row persists. On insert
/// error we `warn!` and move on. A no-op when no DB is configured. The only cost
/// added to the request path is the `Arc` clone and the spawn.
fn record_authz_decision(db: &Option<Arc<Database>>, record: AuthzAuditRecord) {
    let Some(db) = db else {
        return;
    };
    let db = db.clone();
    tokio::spawn(async move {
        if let Err(e) = db.log_authz_decision(&record).await {
            warn!(error = %e, "authz audit write failed (best-effort, ignored)");
        }
    });
}

/// Build the `403 Forbidden` JSON response used when an `Authorize` hook denies:
/// `{"error":"forbidden","message":<msg>}`. Mirrors the 429 budget response
/// shape but with a 403 status.
fn forbidden_response(msg: &str) -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"error": "forbidden", "message": msg}).to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::FORBIDDEN.into_response())
}

/// Build the `400 Bad Request` JSON response used when a `Guardrail` hook blocks:
/// `{"error":"guardrail_blocked","message":<msg>}`. The guardrail is a
/// content-level rejection, so 400 is the closest semantic match (the request
/// itself is unacceptable) while staying distinct from 403 authorization denials.
fn guardrail_blocked_response(msg: &str) -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"error": "guardrail_blocked", "message": msg}).to_string(),
        ))
        .unwrap_or_else(|_| StatusCode::BAD_REQUEST.into_response())
}

/// Read the lifetime (all-time) usage a lookup registry pre-resolved for
/// `user_id`. The key is `usage_budget(<user_id>)`; a missing or unparseable
/// value yields `0` (fail-open). This mirrors the original inline lifetime read
/// and is kept pure so it can be unit-tested without the pipeline.
fn lifetime_usage_from_lookups(lookups: &HashMap<String, String>, user_id: &str) -> u64 {
    let lookup_key = format!("usage_budget({user_id})");
    lookups
        .get(&lookup_key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Outcome of a budget-usage read. `Known` carries the window total; `Unavailable`
/// means neither the shared Redis counter nor the Postgres fallback could be
/// consulted — the caller decides deny-vs-degrade per the scope's outage posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetUsage {
    Known(u64),
    Unavailable,
}

/// Resolve current budget usage for a `MaxTokenBudget` hook.
///
/// - `Lifetime` → the value the lookup registry pre-resolved into
///   `template_ctx.lookups` (unchanged all-time behavior); always `Known`.
/// - windowed → the shared Redis window counter when a rate limiter is present,
///   otherwise the Postgres time-bounded sum fallback. When BOTH are
///   unavailable (Redis error → DB error / no DB) the read is `Unavailable` and
///   the caller applies the scope's outage posture (Agent fails closed).
async fn resolve_budget_usage(
    state: &Arc<AppState>,
    config: &MaxTokenBudgetConfig,
    user_id: &str,
    template_ctx: &TemplateContext,
) -> BudgetUsage {
    if config.window == BudgetWindow::Lifetime {
        return BudgetUsage::Known(lifetime_usage_from_lookups(
            &template_ctx.lookups,
            user_id,
        ));
    }

    // Windowed: prefer the shared Redis counter, fall back to Postgres.
    #[cfg(feature = "redis-l2")]
    {
        if let Some(ref limiter) = state.rate_limiter {
            match limiter
                .get_budget(config.scope, user_id, config.window)
                .await
            {
                Ok(used) => return BudgetUsage::Known(used),
                Err(e) => {
                    warn!(error = %e, user_id, "windowed budget Redis read failed — falling back to DB");
                }
            }
        }
    }

    resolve_budget_usage_from_db(state, config, user_id).await
}

/// Postgres fallback for windowed budget usage. `Unavailable` when the DB read
/// errors or no DB is configured (the caller decides deny-vs-degrade per scope).
async fn resolve_budget_usage_from_db(
    state: &Arc<AppState>,
    config: &MaxTokenBudgetConfig,
    user_id: &str,
) -> BudgetUsage {
    let Some(interval) = config.window.pg_interval() else {
        // No fixed interval (Lifetime only) — treated as 0 usage, not an outage.
        return BudgetUsage::Known(0);
    };
    let Some(ref db) = state.db else {
        // No shared limiter AND no DB — the usage backend is unavailable.
        return BudgetUsage::Unavailable;
    };
    match db.get_user_token_total_windowed(user_id, interval).await {
        Ok(total) => BudgetUsage::Known(total.max(0) as u64),
        Err(e) => {
            warn!(error = %e, user_id, "windowed budget backend read failed — usage unavailable");
            BudgetUsage::Unavailable
        }
    }
}

/// Apply body transform overrides to a JSON body.
///
/// Sets the specified fields in the JSON object. If the body is not valid JSON,
/// returns the original bytes unchanged.
fn apply_body_transforms(body: &Bytes, overrides: &HashMap<String, String>) -> Bytes {
    let mut value: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.clone(),
    };

    if let Value::Object(ref mut map) = value {
        for (field, val) in overrides {
            // Try to parse as JSON, fall back to string
            let json_val = serde_json::from_str(val).unwrap_or_else(|_| Value::String(val.clone()));
            map.insert(field.clone(), json_val);
        }
    }

    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_transform_sets_field() {
        let body = Bytes::from(r#"{"model":"gpt-4","user":"old"}"#);
        let mut overrides = HashMap::new();
        overrides.insert("user".to_string(), "new-user".to_string());
        overrides.insert("temperature".to_string(), "0.7".to_string());

        let result = apply_body_transforms(&body, &overrides);
        let parsed: Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(parsed["user"], "new-user");
        assert_eq!(parsed["temperature"], 0.7);
        assert_eq!(parsed["model"], "gpt-4"); // unchanged
    }

    #[test]
    fn body_transform_non_json_passthrough() {
        let body = Bytes::from("plain text");
        let mut overrides = HashMap::new();
        overrides.insert("field".to_string(), "value".to_string());
        let result = apply_body_transforms(&body, &overrides);
        assert_eq!(result, Bytes::from("plain text"));
    }

    // ── Task 5: budget block decision (pure, no Redis/DB required) ─────────

    fn budget_config(limit: u64, window: BudgetWindow) -> MaxTokenBudgetConfig {
        MaxTokenBudgetConfig {
            limit,
            user_id_expr: "identity.id".to_string(),
            error_message: None,
            window,
            scope: crate::config::types::BudgetScope::User,
        }
    }

    #[test]
    fn budget_exceeded_blocks_at_or_over_limit() {
        assert!(budget_exceeded(100, 100), "exactly at limit must block");
        assert!(budget_exceeded(101, 100), "over limit must block");
    }

    #[test]
    fn budget_exceeded_passes_under_limit() {
        assert!(!budget_exceeded(0, 100));
        assert!(!budget_exceeded(99, 100));
    }

    // ── Budget-backend outage posture (fail-closed for agents) ───────────────

    #[test]
    fn budget_outage_denies_only_for_agent_scope() {
        use crate::config::types::BudgetScope;
        // Agent = hard governance control → deny on a backend outage.
        assert!(budget_outage_denies(BudgetScope::Agent));
        // User/Team = availability-first → degrade (allow) on an outage.
        assert!(!budget_outage_denies(BudgetScope::User));
        assert!(!budget_outage_denies(BudgetScope::Team));
    }

    #[test]
    fn budget_usage_unavailable_is_distinct_from_zero_usage() {
        // The whole point: an outage (`Unavailable`) must NOT be conflated with
        // "0 tokens used" (`Known(0)`), so the check site can fail closed for
        // agents instead of silently allowing.
        assert_ne!(BudgetUsage::Unavailable, BudgetUsage::Known(0));
        assert_eq!(BudgetUsage::Known(0), BudgetUsage::Known(0));
    }

    #[test]
    fn outage_must_deny_covers_agent_scope_and_agent_principal() {
        use crate::auth::identity::{Identity, IdentityKind};
        use crate::config::types::BudgetScope;
        use serde_json::json;

        let human = Identity::default(); // derived_kind == User
        let agent = Identity {
            metadata_public: json!({ "act": { "sub": "u" } }),
            ..Default::default()
        };
        assert_eq!(agent.derived_kind(), IdentityKind::Agent);

        // Agent SCOPE denies regardless of principal.
        assert!(outage_must_deny(BudgetScope::Agent, &human));
        // Actual Agent PRINCIPAL denies even under a mis-scoped user budget
        // (defense in depth — no silent escape via `scope: user`).
        assert!(outage_must_deny(BudgetScope::User, &agent));
        // A human on a user/team budget still degrades (does not deny).
        assert!(!outage_must_deny(BudgetScope::User, &human));
        assert!(!outage_must_deny(BudgetScope::Team, &human));
    }

    #[test]
    fn budget_error_message_uses_override_then_default() {
        let mut cfg = budget_config(10, BudgetWindow::Hour);
        assert_eq!(budget_error_message(&cfg), "token budget exceeded");
        cfg.error_message = Some("custom cap hit".to_string());
        assert_eq!(budget_error_message(&cfg), "custom cap hit");
    }

    #[tokio::test]
    async fn quota_exceeded_response_is_429_json_with_quota_exceeded_error() {
        // Arrange / Act
        let resp = quota_exceeded_response("over the line");
        // Assert — status, content-type, and the stable JSON envelope shape.
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "quota_exceeded");
        assert_eq!(json["message"], "over the line");
    }

    // ── Authorize hook: 403 response + context shape ───────────────────────

    #[tokio::test]
    async fn forbidden_response_is_403_json_with_forbidden_error() {
        let resp = forbidden_response("nope");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "forbidden");
        assert_eq!(json["message"], "nope");
    }

    // -- Guardrail hook: 400 response shape --

    #[tokio::test]
    async fn guardrail_blocked_response_is_400_json_with_guardrail_blocked_error() {
        let resp = guardrail_blocked_response("sensitive content detected");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "guardrail_blocked");
        assert_eq!(json["message"], "sensitive content detected");
    }

    #[test]
    fn build_authz_context_carries_request_attributes() {
        let identity = Identity {
            id: "user-7".to_string(),
            traits: serde_json::json!({"email": "u7@example.com"}),
            ..Default::default()
        };
        let ctx = build_authz_context(&identity, "route-9", "GET", "/api/x");
        assert_eq!(ctx["method"], "GET");
        assert_eq!(ctx["path"], "/api/x");
        assert_eq!(ctx["route_id"], "route-9");
        assert_eq!(ctx["principal_id"], "user-7");
        assert_eq!(ctx["identity"]["email"], "u7@example.com");
    }

    // ── MCP auth-failure responses (Task 3) ────────────────────────────────

    fn parts_with_forwarded_proto(proto: Option<&str>) -> http::request::Parts {
        let (mut parts, _) = http::Request::new(()).into_parts();
        if let Some(p) = proto {
            parts
                .headers
                .insert("x-forwarded-proto", http::HeaderValue::from_str(p).unwrap());
        }
        parts
    }

    #[test]
    fn resource_metadata_url_defaults_to_https() {
        // No x-forwarded-proto → never downgrade the discovery pointer.
        let parts = parts_with_forwarded_proto(None);
        let url = resource_metadata_url("gate.example", &parts);
        assert_eq!(
            url,
            "https://gate.example/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn resource_metadata_url_honors_forwarded_http() {
        let parts = parts_with_forwarded_proto(Some("http"));
        let url = resource_metadata_url("localhost:4456", &parts);
        assert_eq!(
            url,
            "http://localhost:4456/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn resource_metadata_url_ignores_bogus_forwarded_proto() {
        // A garbage proto value must not be reflected verbatim — fall back to https.
        let parts = parts_with_forwarded_proto(Some("javascript"));
        let url = resource_metadata_url("gate.example", &parts);
        assert!(url.starts_with("https://"));
    }

    #[tokio::test]
    async fn mcp_discovery_unauthorized_is_401_with_www_authenticate() {
        let parts = parts_with_forwarded_proto(None);
        let resp = mcp_discovery_unauthorized("gate.example", &parts);
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let wa = resp
            .headers()
            .get(http::header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            wa,
            "Bearer resource_metadata=\"https://gate.example/.well-known/oauth-protected-resource\""
        );
    }

    #[tokio::test]
    async fn mcp_insufficient_scope_is_403_with_scope_challenge() {
        let resp = mcp_insufficient_scope(&["read".to_string(), "write".to_string()]);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let wa = resp
            .headers()
            .get(http::header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(
            wa,
            "Bearer error=\"insufficient_scope\", scope=\"read write\""
        );
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "insufficient_scope");
        assert_eq!(json["scope"], "read write");
    }

    // ── Lifetime path (unchanged): usage read from the pre-resolved lookups ──

    #[test]
    fn lifetime_usage_reads_pre_resolved_lookup_value() {
        let mut lookups = HashMap::new();
        lookups.insert("usage_budget(user-42)".to_string(), "750".to_string());
        assert_eq!(lifetime_usage_from_lookups(&lookups, "user-42"), 750);
    }

    #[test]
    fn lifetime_usage_defaults_to_zero_when_absent_or_unparseable() {
        let mut lookups = HashMap::new();
        lookups.insert("usage_budget(u1)".to_string(), "not-a-number".to_string());
        // Missing key → 0
        assert_eq!(lifetime_usage_from_lookups(&lookups, "absent"), 0);
        // Present but unparseable → 0 (fail-open)
        assert_eq!(lifetime_usage_from_lookups(&lookups, "u1"), 0);
    }

    #[test]
    fn lifetime_decision_blocks_when_pre_resolved_usage_at_limit() {
        // End-to-end of the lifetime arm's pure pieces: read usage, then decide.
        let cfg = budget_config(500, BudgetWindow::Lifetime);
        let mut lookups = HashMap::new();
        lookups.insert("usage_budget(u1)".to_string(), "500".to_string());
        let used = lifetime_usage_from_lookups(&lookups, "u1");
        assert!(budget_exceeded(used, cfg.limit));
    }

    #[test]
    fn lifetime_decision_passes_when_under_limit() {
        let cfg = budget_config(500, BudgetWindow::Lifetime);
        let mut lookups = HashMap::new();
        lookups.insert("usage_budget(u1)".to_string(), "499".to_string());
        let used = lifetime_usage_from_lookups(&lookups, "u1");
        assert!(!budget_exceeded(used, cfg.limit));
    }

    // ── Windowed path: which budgets get metered / which id is used ──────────

    fn ctx_with_identity(id: &str) -> TemplateContext {
        TemplateContext::new(
            serde_json::json!({"id": id, "traits": {}}),
            Value::Null,
            "req-1".to_string(),
            HashMap::new(),
        )
    }

    #[test]
    fn collect_windowed_budgets_excludes_lifetime_and_resolves_id() {
        let hooks = vec![
            PreRequestHook::MaxTokenBudget {
                config: budget_config(1000, BudgetWindow::Lifetime),
            },
            PreRequestHook::MaxTokenBudget {
                config: budget_config(50, BudgetWindow::Hour),
            },
        ];
        let ctx = ctx_with_identity("user-9");
        let collected = collect_windowed_budgets(&hooks, &ctx);
        // Only the windowed (Hour) budget is collected; Lifetime is ledger-only.
        assert_eq!(collected.len(), 1);
        let (scope, window, id) = &collected[0];
        assert_eq!(*scope, crate::config::types::BudgetScope::User);
        assert_eq!(*window, BudgetWindow::Hour);
        assert_eq!(
            id, "user-9",
            "id matches what the pre-request check resolves"
        );
    }

    #[test]
    fn collect_windowed_budgets_empty_when_only_lifetime() {
        let hooks = vec![PreRequestHook::MaxTokenBudget {
            config: budget_config(1000, BudgetWindow::Lifetime),
        }];
        let ctx = ctx_with_identity("u1");
        assert!(collect_windowed_budgets(&hooks, &ctx).is_empty());
    }

    #[test]
    fn agent_scoped_budget_collects_under_agent_scope_and_keys_the_agent_id() {
        // An operator declaring `scope: agent` budgets the agent identity under
        // the Agent scope, so incr_budget writes flint:budget:agent:{id}:… —
        // distinct from the user counter. This is what makes a delegated agent's
        // spend count against its Agent budget, not its User.
        let mut cfg = budget_config(50, BudgetWindow::Hour);
        cfg.scope = crate::config::types::BudgetScope::Agent;
        let hooks = vec![PreRequestHook::MaxTokenBudget { config: cfg }];
        let ctx = ctx_with_identity("agent-7");
        let collected = collect_windowed_budgets(&hooks, &ctx);
        assert_eq!(collected.len(), 1);
        let (scope, _window, id) = &collected[0];
        assert_eq!(*scope, crate::config::types::BudgetScope::Agent);
        assert_eq!(id, "agent-7");
        assert_eq!(scope.tag(), "agent"); // → flint:budget:agent:agent-7:hour
    }
}
