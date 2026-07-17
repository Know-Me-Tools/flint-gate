//! OAuth 2.0 Client Credentials grant (RFC 6749 §4.4) — service-to-service
//! (non-human) identity. A client authenticates with `client_id` +
//! `client_secret` and receives a short-lived gateway-minted access token
//! carrying its `client_id`, granted scopes, and audience.
//!
//! Where Ory Hydra is the authorization server, prefer Hydra's own
//! client-credentials; this is the gateway-local path for clients flint-gate
//! itself issues.

use crate::auth::identity::Identity;
use crate::auth::jwt_mint::SharedJwtMinter;
use crate::db::Database;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// RFC 6749 client-credentials grant type.
pub const GRANT_TYPE_CLIENT_CREDENTIALS: &str = "client_credentials";

/// Client-credentials request parameters (form-encoded).
#[derive(Debug, Deserialize)]
pub struct ClientCredentialsRequest {
    pub grant_type: String,
    pub client_id: String,
    pub client_secret: String,
    /// Space-delimited requested scopes. Must be a subset of the client's grant.
    #[serde(default)]
    pub scope: Option<String>,
}

/// A client-credentials failure mapped to an OAuth 2.0 error response.
#[derive(Debug, PartialEq, Eq)]
pub enum ClientCredentialsError {
    UnsupportedGrantType(String),
    /// Unknown client or wrong secret — RFC 6749 `invalid_client`.
    InvalidClient,
    /// Requested scope exceeds the client's granted scopes.
    InvalidScope(String),
    MintFailed(String),
    /// No client store (database) is configured.
    NotConfigured,
}

impl ClientCredentialsError {
    pub fn oauth_code(&self) -> &'static str {
        match self {
            ClientCredentialsError::UnsupportedGrantType(_) => "unsupported_grant_type",
            ClientCredentialsError::InvalidClient => "invalid_client",
            ClientCredentialsError::InvalidScope(_) => "invalid_scope",
            ClientCredentialsError::MintFailed(_) => "server_error",
            ClientCredentialsError::NotConfigured => "temporarily_unavailable",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ClientCredentialsError::UnsupportedGrantType(m)
            | ClientCredentialsError::InvalidScope(m)
            | ClientCredentialsError::MintFailed(m) => m.clone(),
            ClientCredentialsError::InvalidClient => "invalid client credentials".to_string(),
            ClientCredentialsError::NotConfigured => "client store not configured".to_string(),
        }
    }
}

/// Restrict `requested` to a subset of the client's `granted` scopes, failing
/// closed on any scope the client does not hold. Absent request → all granted.
/// Shares the exact subset semantics used by token exchange (single source of
/// truth would be nice, but the error type differs; kept small + tested).
pub fn restrict_scopes(
    requested: Option<&str>,
    granted: &[String],
) -> Result<Vec<String>, ClientCredentialsError> {
    let Some(requested) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(granted.to_vec());
    };
    let mut out: Vec<String> = Vec::new();
    for scope in requested.split_whitespace() {
        if granted.iter().any(|g| g == scope) {
            // De-duplicate so a repeated request scope doesn't bloat the claim.
            if !out.iter().any(|s| s == scope) {
                out.push(scope.to_string());
            }
        } else {
            return Err(ClientCredentialsError::InvalidScope(format!(
                "requested scope {scope:?} exceeds the client's grant"
            )));
        }
    }
    Ok(out)
}

/// The RFC 6749 §5.1 success response for a minted access token.
pub fn client_credentials_response(
    access_token: String,
    granted: &[String],
    ttl_seconds: Option<u64>,
) -> Value {
    let mut resp = json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "scope": granted.join(" "),
    });
    if let (Some(ttl), Value::Object(map)) = (ttl_seconds, &mut resp) {
        map.insert("expires_in".to_string(), json!(ttl));
    }
    resp
}

/// End-to-end client-credentials grant: verify the client → restrict scopes →
/// mint a service token (`client_id`, `scope`, `aud`) via the shared minter.
/// Fail-closed: unknown client / bad secret / scope-exceed / no minter all deny.
pub async fn client_credentials_grant(
    req: &ClientCredentialsRequest,
    db: &Arc<Database>,
    minter: &SharedJwtMinter,
    ttl_seconds: Option<u64>,
) -> Result<Value, ClientCredentialsError> {
    if req.grant_type != GRANT_TYPE_CLIENT_CREDENTIALS {
        return Err(ClientCredentialsError::UnsupportedGrantType(format!(
            "unsupported grant_type: {}",
            req.grant_type
        )));
    }

    let client = db
        .verify_client_credentials(&req.client_id, &req.client_secret)
        .await
        .map_err(|e| ClientCredentialsError::MintFailed(e.to_string()))?
        .ok_or(ClientCredentialsError::InvalidClient)?;

    let granted = restrict_scopes(req.scope.as_deref(), &client.scopes)?;

    // The service token's subject IS the client_id (non-human identity).
    let identity = Identity {
        id: client.client_id.clone(),
        ..Default::default()
    };
    // Stamp the gateway's `flint_kind` marker so this token authorizes as a
    // Service on re-verification (a bare `client_id` claim is NOT trusted for
    // kind classification — see Identity::derived_kind).
    let mut additional = json!({
        crate::auth::identity::FLINT_KIND_CLAIM: "service",
        "client_id": client.client_id,
        "scope": granted.join(" "),
    });
    if let (Some(aud), Value::Object(map)) = (&client.audience, &mut additional) {
        map.insert("aud".to_string(), json!(aud));
    }

    let guard = minter.read().await;
    let minter = guard
        .as_ref()
        .ok_or_else(|| ClientCredentialsError::MintFailed("JWT minter not configured".into()))?;
    let token = minter
        .mint(&identity, Some(&additional), ttl_seconds)
        .map_err(|e| ClientCredentialsError::MintFailed(e.to_string()))?;

    Ok(client_credentials_response(token, &granted, ttl_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted() -> Vec<String> {
        vec!["svc.read".into(), "svc.write".into()]
    }

    #[test]
    fn restrict_inherits_when_none_requested() {
        assert_eq!(restrict_scopes(None, &granted()).unwrap(), granted());
    }

    #[test]
    fn restrict_allows_subset() {
        assert_eq!(
            restrict_scopes(Some("svc.read"), &granted()).unwrap(),
            vec!["svc.read"]
        );
    }

    #[test]
    fn restrict_denies_scope_beyond_grant() {
        let err = restrict_scopes(Some("svc.read svc.admin"), &granted()).unwrap_err();
        assert_eq!(err.oauth_code(), "invalid_scope");
    }

    #[test]
    fn restrict_dedupes_repeated_scopes() {
        assert_eq!(
            restrict_scopes(Some("svc.read svc.read"), &granted()).unwrap(),
            vec!["svc.read"]
        );
    }

    #[test]
    fn wrong_grant_type_code() {
        assert_eq!(
            ClientCredentialsError::UnsupportedGrantType("x".into()).oauth_code(),
            "unsupported_grant_type"
        );
    }

    #[test]
    fn invalid_client_code() {
        assert_eq!(ClientCredentialsError::InvalidClient.oauth_code(), "invalid_client");
    }

    #[test]
    fn response_includes_expires_in_when_ttl_set() {
        let resp = client_credentials_response("t.o.k".into(), &["svc.read".into()], Some(300));
        assert_eq!(resp["token_type"], "Bearer");
        assert_eq!(resp["expires_in"], 300);
        assert_eq!(resp["scope"], "svc.read");
    }

    #[test]
    fn response_omits_expires_in_when_ttl_absent() {
        let resp = client_credentials_response("t.o.k".into(), &[], None);
        assert!(resp.get("expires_in").is_none());
    }
}
