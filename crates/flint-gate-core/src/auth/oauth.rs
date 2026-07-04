//! Unified OAuth 2.0 token endpoint (`POST /oauth/token`) that dispatches by
//! `grant_type`, plus the RFC 7662 introspection endpoint. Both mount on the
//! proxy port. Each grant is independently gated by config; a grant that is not
//! enabled returns `unsupported_grant_type`.

use crate::auth::client_credentials::{
    client_credentials_grant, ClientCredentialsError, ClientCredentialsRequest,
    GRANT_TYPE_CLIENT_CREDENTIALS,
};
use crate::auth::introspect::{delegate_to_hydra, IntrospectRequest, TokenVerifier};
use crate::auth::jwt_mint::SharedJwtMinter;
use crate::auth::token_exchange::{
    exchange, TokenExchangeRequest, GRANT_TYPE_TOKEN_EXCHANGE,
};
use crate::auth::Authenticator;
use crate::db::Database;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// State for the unified `/oauth/token` + `/oauth/introspect` endpoints. Each
/// capability is `Option` — absent means "not enabled".
#[derive(Clone)]
pub struct OAuthState {
    pub minter: SharedJwtMinter,
    /// Token-exchange verifier + TTL (RFC 8693). `None` disables that grant.
    pub token_exchange: Option<(Arc<dyn Authenticator>, Option<u64>)>,
    /// Client store + service-token TTL for client-credentials. `None` disables it.
    pub client_credentials: Option<(Arc<Database>, Option<u64>)>,
    /// Verifier + optional Hydra delegate for introspection. `None` disables it.
    pub introspection: Option<IntrospectionState>,
}

/// Introspection dependencies.
#[derive(Clone)]
pub struct IntrospectionState {
    pub verifier: TokenVerifier,
    pub http_client: reqwest::Client,
    /// Hydra admin URL for delegating opaque-token introspection (seam, off unless set).
    pub hydra_admin_url: Option<String>,
}

fn oauth_error(status: StatusCode, code: &str, description: String) -> Response {
    // Never leak internal (e.g. database) error detail to the client on a 5xx —
    // log it server-side and return a generic description. Client (4xx) errors
    // keep their specific, safe message.
    let description = if status.is_server_error() {
        tracing::error!(error = %code, detail = %description, "oauth endpoint server error");
        "internal server error".to_string()
    } else {
        description
    };
    (
        status,
        Json(json!({ "error": code, "error_description": description })),
    )
        .into_response()
}

/// `POST /oauth/token` — dispatch by `grant_type`. The raw form is parsed once
/// into a map so the grant is selected before deserializing into the concrete
/// per-grant request type.
pub async fn token_endpoint(
    State(state): State<OAuthState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let grant = form.get("grant_type").map(String::as_str).unwrap_or("");

    match grant {
        GRANT_TYPE_TOKEN_EXCHANGE => {
            let Some((verifier, ttl)) = &state.token_exchange else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "unsupported_grant_type",
                    "token exchange is not enabled".into(),
                );
            };
            let req = TokenExchangeRequest {
                grant_type: grant.to_string(),
                subject_token: form.get("subject_token").cloned().unwrap_or_default(),
                subject_token_type: form.get("subject_token_type").cloned(),
                scope: form.get("scope").cloned(),
                resource: form.get("resource").cloned(),
                audience: form.get("audience").cloned(),
                actor_token: form.get("actor_token").cloned(),
            };
            match exchange(&req, verifier, &state.minter, *ttl).await {
                Ok(body) => (StatusCode::OK, Json(body)).into_response(),
                Err(e) => {
                    let status = match &e {
                        crate::auth::token_exchange::ExchangeError::MintFailed(_) => {
                            StatusCode::INTERNAL_SERVER_ERROR
                        }
                        _ => StatusCode::BAD_REQUEST,
                    };
                    oauth_error(status, e.oauth_code(), e.message())
                }
            }
        }
        GRANT_TYPE_CLIENT_CREDENTIALS => {
            let Some((db, ttl)) = &state.client_credentials else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "unsupported_grant_type",
                    "client_credentials is not enabled".into(),
                );
            };
            let req = ClientCredentialsRequest {
                grant_type: grant.to_string(),
                client_id: form.get("client_id").cloned().unwrap_or_default(),
                client_secret: form.get("client_secret").cloned().unwrap_or_default(),
                scope: form.get("scope").cloned(),
            };
            match client_credentials_grant(&req, db, &state.minter, *ttl).await {
                Ok(body) => (StatusCode::OK, Json(body)).into_response(),
                Err(e) => {
                    let status = match &e {
                        ClientCredentialsError::MintFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
                        ClientCredentialsError::InvalidClient => StatusCode::UNAUTHORIZED,
                        _ => StatusCode::BAD_REQUEST,
                    };
                    oauth_error(status, e.oauth_code(), e.message())
                }
            }
        }
        other => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            format!("unsupported grant_type: {other}"),
        ),
    }
}

/// `POST /oauth/introspect` (RFC 7662). Verifies gateway-minted tokens locally;
/// when local verification reports inactive AND a Hydra delegate is configured,
/// forwards to Hydra (which owns introspection for its own opaque tokens).
pub async fn introspect_endpoint(
    State(state): State<OAuthState>,
    Form(req): Form<IntrospectRequest>,
) -> Response {
    let Some(intro) = &state.introspection else {
        return oauth_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "introspection is not enabled".into(),
        );
    };
    let _ = &req.token_type_hint; // accepted, not required

    let local = intro.verifier.introspect(&req.token);
    if local.get("active") == Some(&Value::Bool(true)) {
        return (StatusCode::OK, Json(local)).into_response();
    }
    // Local says inactive — try the Hydra delegate if configured.
    if let Some(hydra_url) = &intro.hydra_admin_url {
        let delegated = delegate_to_hydra(&intro.http_client, hydra_url, &req.token).await;
        return (StatusCode::OK, Json(delegated)).into_response();
    }
    (StatusCode::OK, Json(local)).into_response()
}
