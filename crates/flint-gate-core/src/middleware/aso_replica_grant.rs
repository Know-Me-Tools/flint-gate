//! Fresh ASO replica grant resolution and typed downstream token minting.

use crate::{
    auth::{
        jwt_mint::{ReplicaMintGrant, ASO_PROJECTION_REVISION},
        AuthMethod, Identity, JwtMinter,
    },
    config::types::AsoReplicaGrantConfig,
};
use chrono::{DateTime, Utc};
use http::{HeaderMap, StatusCode};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::LazyLock, time::Duration};
use uuid::Uuid;

static GRANT_CLIENT: LazyLock<Result<reqwest::Client, ()>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| ())
});

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectionId {
    Cases,
    CaseEvidence,
    EvidenceStates,
    EvidenceCitations,
    Documents,
}

impl ProjectionId {
    fn claim_name(&self) -> &'static str {
        match self {
            Self::Cases => "cases",
            Self::CaseEvidence => "case_evidence",
            Self::EvidenceStates => "evidence_states",
            Self::EvidenceCitations => "evidence_citations",
            Self::Documents => "documents",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProjectionMarker {
    id: ProjectionId,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplicaGrantResponse {
    identity_id: Uuid,
    originating_session_id: Uuid,
    practice_id: Uuid,
    authorization_revision: String,
    projection_revision: u32,
    expires_at: DateTime<Utc>,
    projections: Vec<ProjectionMarker>,
}

pub(super) async fn mint(
    config: &AsoReplicaGrantConfig,
    auth_method: &AuthMethod,
    identity: &Identity,
    request_uri: &str,
    original_headers: &HeaderMap,
    minter: &JwtMinter,
) -> Result<String, StatusCode> {
    if !matches!(auth_method, AuthMethod::KratosSession) {
        return Err(StatusCode::FORBIDDEN);
    }
    let expected_identity = Uuid::parse_str(&identity.id).map_err(|_| StatusCode::FORBIDDEN)?;
    let expected_session = identity
        .session_id
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    let practice = selected_practice(request_uri)?;
    let credentials = super::aso_clinical_authorize::credential_headers(original_headers)?;
    let mut url = super::aso_clinical_authorize::callback_url(&config.url)?;
    if let Some(practice) = practice {
        url.query_pairs_mut()
            .append_pair("practiceId", &practice.to_string());
    }
    let client = GRANT_CLIENT
        .as_ref()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = client
        .get(url)
        .headers(credentials)
        .send()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED => return Err(StatusCode::UNAUTHORIZED),
        StatusCode::FORBIDDEN => return Err(StatusCode::FORBIDDEN),
        _ => return Err(StatusCode::SERVICE_UNAVAILABLE),
    }
    let grant = response
        .json::<ReplicaGrantResponse>()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if grant.identity_id != expected_identity
        || grant.originating_session_id != expected_session
        || practice.is_some_and(|value| value != grant.practice_id)
        || grant.projection_revision != ASO_PROJECTION_REVISION
        || grant.authorization_revision.trim().is_empty()
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let projection_ids = grant
        .projections
        .iter()
        .map(|projection| projection.id.claim_name().to_owned())
        .collect::<Vec<_>>();
    let unique = projection_ids.iter().collect::<BTreeSet<_>>();
    if projection_ids.len() != 5 || unique.len() != 5 {
        return Err(StatusCode::FORBIDDEN);
    }
    minter
        .mint_replica(&ReplicaMintGrant {
            subject: grant.identity_id,
            audience: config.audience.clone(),
            tenant_id: grant.practice_id,
            authorization_revision: grant.authorization_revision,
            originating_session_id: grant.originating_session_id,
            projection_revision: grant.projection_revision,
            projection_ids,
            expires_at: grant.expires_at,
            max_ttl_seconds: config.max_ttl_seconds,
        })
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

fn selected_practice(uri: &str) -> Result<Option<Uuid>, StatusCode> {
    let Some((_, query)) = uri.split_once('?') else {
        return Ok(None);
    };
    let values = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == "practiceId")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => Uuid::parse_str(value)
            .map(Some)
            .map_err(|_| StatusCode::BAD_REQUEST),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth::identity::IdentityKind, config::types::JwtConfig};
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    use serde_json::{json, Value};
    use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

    async fn minter() -> JwtMinter {
        JwtMinter::from_config(&JwtConfig {
            signing_algorithm: "HS256".into(),
            signing_key_secret: Some("test-secret-key-minimum-length".into()),
            issuer: "https://gate.test".into(),
            default_ttl_seconds: 300,
            ..Default::default()
        })
        .await
        .unwrap()
    }

    fn identity() -> Identity {
        Identity {
            id: Uuid::from_u128(1).to_string(),
            kind: IdentityKind::User,
            session_id: Some(Uuid::from_u128(2).to_string()),
            traits: json!({"scope": "all", "table": "aso.patients", "role": "admin"}),
            ..Default::default()
        }
    }

    fn grant(revision: u32) -> Value {
        json!({
            "identityId": Uuid::from_u128(1),
            "originatingSessionId": Uuid::from_u128(2),
            "practiceId": Uuid::from_u128(3),
            "authorizationRevision": "membership:synthetic",
            "projectionRevision": revision,
            "expiresAt": Utc::now() + chrono::Duration::minutes(5),
            "projections": [
                {"id": "cases"}, {"id": "case_evidence"},
                {"id": "evidence_states"}, {"id": "evidence_citations"},
                {"id": "documents"}
            ]
        })
    }

    fn config(server: &MockServer) -> AsoReplicaGrantConfig {
        AsoReplicaGrantConfig {
            url: format!("{}/api/session/replica-grant", server.uri()),
            audience: "frf-gateway".into(),
            max_ttl_seconds: 60,
        }
    }

    #[tokio::test]
    async fn caller_fields_and_forged_identity_data_cannot_enter_replica_claims() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("GET"))
            .and(matchers::path("/api/session/replica-grant"))
            .and(matchers::query_param(
                "practiceId",
                Uuid::from_u128(3).to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant(1)))
            .expect(1)
            .mount(&server)
            .await;
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "ory_kratos_session=synthetic".parse().unwrap());
        headers.insert("x-flint-role", "admin".parse().unwrap());
        headers.insert("x-aso-columns", "patient_id".parse().unwrap());
        let token = mint(
            &config(&server),
            &AuthMethod::KratosSession,
            &identity(),
            &format!(
                "/v1/shape?practiceId={}&table=aso.patients&where=true&columns=patient_id",
                Uuid::from_u128(3)
            ),
            &headers,
            &minter().await,
        )
        .await
        .unwrap();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&["frf-gateway"]);
        validation.set_issuer(&["https://gate.test"]);
        let claims = decode::<Value>(
            &token,
            &DecodingKey::from_secret("test-secret-key-minimum-length".as_bytes()),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims["scope"], "aso.replica.read");
        assert_eq!(claims["tenant_id"], Uuid::from_u128(3).to_string());
        for rejected in ["table", "where", "columns", "role", "patient_id"] {
            assert!(claims.get(rejected).is_none());
        }
    }

    #[tokio::test]
    async fn service_identity_and_wrong_revision_are_refused() {
        let server = MockServer::start().await;
        assert_eq!(
            mint(
                &config(&server),
                &AuthMethod::BearerJwt,
                &identity(),
                "/v1/shape",
                &HeaderMap::new(),
                &minter().await,
            )
            .await,
            Err(StatusCode::FORBIDDEN)
        );
        Mock::given(matchers::method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant(99)))
            .mount(&server)
            .await;
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "synthetic".parse().unwrap());
        assert_eq!(
            mint(
                &config(&server),
                &AuthMethod::KratosSession,
                &identity(),
                "/v1/shape",
                &headers,
                &minter().await,
            )
            .await,
            Err(StatusCode::FORBIDDEN)
        );
    }
}
