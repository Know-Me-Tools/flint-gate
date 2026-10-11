//! AFC C02 production-path integration fixture.
//!
//! This matrix enters through the authenticated admin router, exercises the
//! Cedar governed-effect provider and durable memory challenge store, and
//! leaves effect execution outside Gate.

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use flint_gate_core::admin::{admin_router_with_auth, auth::AdminAuthenticator, AdminState};
use flint_gate_core::approval::ApprovalManager;
use flint_gate_core::auth::{AuthError, AuthMethod, AuthResult, Authenticator, Identity};
use flint_gate_core::authz::{AuthzEngine, PolicyRecord};
use flint_gate_core::cache::GateCache;
use flint_gate_core::config::types::{
    AdminAuthConfig, AuthProviderConfig, CacheConfig, GateConfig, JwtAuthConfig,
};
use flint_gate_core::governed_effect::{CedarGovernedEffectAuthority, MemoryChallengeStore};
use flint_gate_core::proxy::Router as GateRouter;
use http::request::Parts;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const P1_ISSUER: &str = "https://p1.example";
const P1_SUBJECT: &str = "uar-host-1";
const EFFECT_ISSUER: &str = "https://identity.example";

struct P1Authenticator;

#[async_trait]
impl Authenticator for P1Authenticator {
    async fn authenticate(&self, parts: &Parts) -> Result<AuthResult, AuthError> {
        let credential = parts
            .headers
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if credential != Some("Bearer p1-authority") {
            return Err(AuthError::Unauthorized(
                "verified P1 execution owner required".to_owned(),
            ));
        }
        Ok(AuthResult {
            identity: Identity::anonymous(P1_SUBJECT),
            method: AuthMethod::BearerJwt,
        })
    }
}

fn policy(id: &str, text: &str) -> PolicyRecord {
    PolicyRecord {
        id: id.to_owned(),
        policy_text: text.to_owned(),
        schema_json: None,
        entities_json: None,
    }
}

fn authority_engine(challenge_reason: &str) -> Arc<AuthzEngine> {
    Arc::new(
        AuthzEngine::from_records(&[
            policy(
                "challenge",
                &format!(
                    r#"@require_approval("{challenge_reason}")
                    permit(principal, action == Action::"afc:challenge/v1", resource);"#
                ),
            ),
            policy(
                "permit",
                r#"permit(principal, action == Action::"afc:permit/v1", resource);"#,
            ),
        ])
        .expect("integration policy set compiles"),
    )
}

fn app(engine: Arc<AuthzEngine>, challenges: Arc<MemoryChallengeStore>) -> Router {
    let mut gate_config = GateConfig::default();
    gate_config.server.admin_auth = Some(AdminAuthConfig {
        provider: AuthProviderConfig::Jwt(JwtAuthConfig {
            jwks_url: "https://p1.example/.well-known/jwks.json".to_owned(),
            issuer: Some(P1_ISSUER.to_owned()),
            audience: Some("flint-gate-admin".to_owned()),
            leeway_seconds: 5,
        }),
    });
    let config = Arc::new(tokio::sync::RwLock::new(gate_config.clone()));
    let router = Arc::new(tokio::sync::RwLock::new(GateRouter::from_config(
        &gate_config,
    )));
    let state = AdminState {
        cache: Arc::new(GateCache::from_config(&CacheConfig::default())),
        db: None,
        router,
        config,
        authz: Arc::clone(&engine),
        approval_manager: Arc::new(ApprovalManager::new()),
        governed_effects: Arc::new(CedarGovernedEffectAuthority::new(engine, challenges)),
        admin_events: None,
    };
    let authenticator: AdminAuthenticator = Arc::new(P1Authenticator);
    admin_router_with_auth(state, Some(authenticator), None)
}

fn governed_request(action: &str) -> Value {
    json!({
        "protocol": "afc.governed-effect/1",
        "effect_id": Uuid::new_v4(),
        "invocation_id": Uuid::new_v4(),
        "action": {
            "namespace": "afc",
            "name": action,
            "version": "v1"
        },
        "resource": {
            "kind": "http",
            "destination": "https://api.example/customers/42"
        },
        "payload": {
            "algorithm": "sha256",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "identity": {
            "issuer": EFFECT_ISSUER,
            "subject": "agent-42",
            "subject_kind": "agent",
            "actor": "operator-7",
            "audience": ["https://api.example"],
            "tenant": "tenant-9",
            "identity_revision": "identity-r7",
            "verified": true,
            "revoked": false
        },
        "grants": [{
            "issuer": EFFECT_ISSUER,
            "grant_id": "grant-7",
            "revision": "grant-r3",
            "active": true
        }],
        "lease": {
            "lease_id": "lease-11",
            "revision": "lease-r4",
            "expires_at": Utc::now() + Duration::minutes(10),
            "active": true
        },
        "budget": {
            "budget_id": "budget-5",
            "revision": "budget-r2",
            "reservation_id": "reservation-17",
            "expires_at": Utc::now() + Duration::minutes(10),
            "active": true
        },
        "epochs": {
            "runtime": "runtime-12",
            "host": "host-6",
            "catalog": "catalog-8"
        }
    })
}

async fn post(app: &Router, uri: &str, credential: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(http::header::AUTHORIZATION, credential)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, body)
}

async fn evaluate(app: &Router, request: Value) -> (StatusCode, Value) {
    post(
        app,
        "/authority/effects/evaluate",
        "Bearer p1-authority",
        request,
    )
    .await
}

fn assert_denied(response: &(StatusCode, Value), reason: &str) {
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["disposition"], "deny");
    assert_eq!(response.1["reason"], reason);
    assert!(response.1["challenge_id"].is_null());
}

fn decision_uri(challenge_id: &str) -> String {
    let issuer: String = url::form_urlencoded::byte_serialize(EFFECT_ISSUER.as_bytes()).collect();
    format!("/authority/effects/{issuer}/{challenge_id}/decision")
}

#[tokio::test]
async fn authenticated_governed_effect_authority_matrix() {
    let challenges = MemoryChallengeStore::new();
    let engine = authority_engine("operator confirmation");
    let admin_app = app(Arc::clone(&engine), Arc::clone(&challenges));

    // Authentication runs on the real protected admin-router path.
    let unauthenticated = post(
        &admin_app,
        "/authority/effects/evaluate",
        "Bearer forged-host",
        governed_request("challenge"),
    )
    .await;
    assert_eq!(unauthenticated.0, StatusCode::UNAUTHORIZED);

    // Direct Cedar permit and default-deny carry no challenge or execution authority.
    let permitted = evaluate(&admin_app, governed_request("permit")).await;
    assert_eq!(permitted.0, StatusCode::OK);
    assert_eq!(permitted.1["disposition"], "permit");
    assert!(permitted.1["challenge_id"].is_null());
    let denied = evaluate(&admin_app, governed_request("deny")).await;
    assert_denied(&denied, "policy_denied");

    // A client cannot forge Gate's policy binding into the request contract.
    let mut client_policy = governed_request("challenge");
    client_policy["policy"] = json!({
        "set_id": "attacker",
        "revision": "attacker",
        "digest": "sha256:attacker"
    });
    assert_eq!(
        evaluate(&admin_app, client_policy).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Forged or invalid identity, issuer/grant, and audience facts fail closed.
    let mut forged_identity = governed_request("challenge");
    forged_identity["identity"]["verified"] = json!(false);
    assert_denied(
        &evaluate(&admin_app, forged_identity).await,
        "invalid_or_revoked_identity",
    );
    let mut forged_issuer = governed_request("challenge");
    forged_issuer["grants"][0]["issuer"] = json!("https://attacker.example");
    assert_denied(
        &evaluate(&admin_app, forged_issuer).await,
        "inactive_or_invalid_grant",
    );
    let mut forged_audience = governed_request("challenge");
    forged_audience["identity"]["audience"] = json!([]);
    assert_denied(
        &evaluate(&admin_app, forged_audience).await,
        "invalid_identity_scope",
    );
    let mut duplicate_audience = governed_request("challenge");
    duplicate_audience["identity"]["audience"] =
        json!(["https://api.example", "https://api.example"]);
    assert_denied(
        &evaluate(&admin_app, duplicate_audience).await,
        "invalid_identity_scope",
    );

    // Exact duplicate evaluation deduplicates; a distinct issuer gets a distinct identity.
    let request = governed_request("challenge");
    let challenged = evaluate(&admin_app, request.clone()).await;
    assert_eq!(challenged.0, StatusCode::OK);
    assert_eq!(challenged.1["disposition"], "challenge");
    let challenge_id = challenged.1["challenge_id"].as_str().unwrap();
    let duplicate = evaluate(&admin_app, request.clone()).await;
    assert_eq!(duplicate.1["challenge_id"], challenge_id);
    let mut other_issuer = request.clone();
    other_issuer["identity"]["issuer"] = json!("https://other-identity.example");
    other_issuer["grants"][0]["issuer"] = json!("https://other-identity.example");
    let distinct = evaluate(&admin_app, other_issuer).await;
    assert_ne!(distinct.1["challenge_id"], challenge_id);

    // An authenticated administrator resolves the durable challenge once.
    let approved_decision_uri = decision_uri(challenge_id);
    let approved = post(
        &admin_app,
        &approved_decision_uri,
        "Bearer p1-authority",
        json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(approved.0, StatusCode::OK);
    let duplicate_decision = post(
        &admin_app,
        &approved_decision_uri,
        "Bearer p1-authority",
        json!({"decision": "approve"}),
    )
    .await;
    assert_eq!(duplicate_decision.0, StatusCode::CONFLICT);

    let revalidate_uri = "/authority/effects/revalidate";
    let exact_revalidation = post(
        &admin_app,
        revalidate_uri,
        "Bearer p1-authority",
        json!({
            "issuer": EFFECT_ISSUER,
            "challenge_id": challenge_id,
            "request": request.clone()
        }),
    )
    .await;
    assert_eq!(exact_revalidation.0, StatusCode::OK);
    assert_eq!(exact_revalidation.1["disposition"], "permit");

    // Payload, resource, grant, and active Gate policy revisions are exact bindings.
    let mut changed_payload = request.clone();
    changed_payload["payload"]["sha256"] =
        json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let changed_payload = post(
        &admin_app,
        revalidate_uri,
        "Bearer p1-authority",
        json!({"issuer": EFFECT_ISSUER, "challenge_id": challenge_id, "request": changed_payload}),
    )
    .await;
    assert_denied(&changed_payload, "authority_binding_changed");

    let mut changed_resource = request.clone();
    changed_resource["resource"]["destination"] = json!("https://api.example/customers/99");
    assert_denied(
        &post(
            &admin_app,
            revalidate_uri,
            "Bearer p1-authority",
            json!({"issuer": EFFECT_ISSUER, "challenge_id": challenge_id, "request": changed_resource}),
        )
        .await,
        "authority_binding_changed",
    );

    let mut stale_grant = request.clone();
    stale_grant["grants"][0]["revision"] = json!("grant-r4");
    assert_denied(
        &post(
            &admin_app,
            revalidate_uri,
            "Bearer p1-authority",
            json!({"issuer": EFFECT_ISSUER, "challenge_id": challenge_id, "request": stale_grant}),
        )
        .await,
        "authority_binding_changed",
    );

    let changed_policy_app = app(
        authority_engine("a newer policy revision"),
        Arc::clone(&challenges),
    );
    let policy_stale = post(
        &changed_policy_app,
        revalidate_uri,
        "Bearer p1-authority",
        json!({
            "issuer": EFFECT_ISSUER,
            "challenge_id": challenge_id,
            "request": request
        }),
    )
    .await;
    assert_denied(&policy_stale, "authority_binding_changed");

    // Revoked approval, expired lease/budget, and inactive grant stay denied.
    let rejected_request = governed_request("challenge");
    let rejected_challenge = evaluate(&admin_app, rejected_request.clone()).await;
    let rejected_id = rejected_challenge.1["challenge_id"].as_str().unwrap();
    let rejected_uri = decision_uri(rejected_id);
    assert_eq!(
        post(
            &admin_app,
            &rejected_uri,
            "Bearer p1-authority",
            json!({"decision": "deny"}),
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_denied(
        &post(
            &admin_app,
            revalidate_uri,
            "Bearer p1-authority",
            json!({"issuer": EFFECT_ISSUER, "challenge_id": rejected_id, "request": rejected_request}),
        )
        .await,
        "approval_not_granted",
    );

    let mut expired_lease = governed_request("challenge");
    expired_lease["lease"]["expires_at"] = json!(Utc::now() - Duration::seconds(1));
    assert_denied(
        &evaluate(&admin_app, expired_lease).await,
        "inactive_or_expired_lease",
    );
    let mut inactive_grant = governed_request("challenge");
    inactive_grant["grants"][0]["active"] = json!(false);
    assert_denied(
        &evaluate(&admin_app, inactive_grant).await,
        "inactive_or_invalid_grant",
    );
    let mut expired_budget = governed_request("challenge");
    expired_budget["budget"]["expires_at"] = json!(Utc::now() - Duration::seconds(1));
    assert_denied(
        &evaluate(&admin_app, expired_budget).await,
        "inactive_expired_or_invalid_budget",
    );
}
