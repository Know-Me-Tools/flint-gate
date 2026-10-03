//! Envoy `ext_authz` check endpoint, driven with Envoy-shaped check requests.
//!
//! Envoy's HTTP ext_authz client keeps the client's method and `Host`,
//! prefixes the client path with `extAuth.http.path`, sends no body and
//! forwards the headers the policy lists. These tests send exactly that to the
//! real handler over the real `AppState`, providers and ES256 minter. Kratos
//! and the inbound JWT issuer's JWKS are wiremock servers; the `api_key` test
//! needs Postgres and is `#[ignore]`d like the repo's other database tests:
//!
//!   DATABASE_URL=postgres://... cargo test -p flint-gate-core --test ext_authz -- --include-ignored

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::Utc;
use flint_gate_core::approval::{durable::MemoryApprovalStore, ApprovalManager};
use flint_gate_core::auth::jwks_publish::{build_jwks, jwks_handler};
use flint_gate_core::auth::{build_authenticators, Identity, JwtMinter};
use flint_gate_core::authz::AuthzEngine;
use flint_gate_core::cache::GateCache;
use flint_gate_core::config::{GateConfig, JwtConfig, LookupRegistry};
use flint_gate_core::db::{Database, JwtSigningKeyPublic};
use flint_gate_core::middleware::ext_authz::{
    ext_authz_handler, ENVOY_HEADERS_TO_REMOVE, EXT_AUTHZ_ROUTES,
};
use flint_gate_core::middleware::AppState;
use flint_gate_core::proxy::Router as GateRouter;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use p256::pkcs8::{EncodePrivateKey, EncodePublicKey};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HOST: &str = "know-me.tools";
const ISSUER: &str = "https://gate.test";
const KID: &str = "gate-test-key";
const KRATOS_USER: &str = "8f6e3c1a-1111-4b2c-9d3e-000000000001";

struct SigningKey {
    private_pem: String,
    public_pem: String,
}

fn es256_key() -> SigningKey {
    let private = p256::SecretKey::random(&mut rand::thread_rng());
    SigningKey {
        private_pem: private
            .to_pkcs8_pem(Default::default())
            .expect("private PEM")
            .to_string(),
        public_pem: private
            .public_key()
            .to_public_key_pem(Default::default())
            .expect("public PEM"),
    }
}

/// Gate's published JWKS for `key`, built by the same code that serves
/// `/.well-known/jwks.json`.
fn gate_jwks(key: &SigningKey) -> Value {
    let row = JwtSigningKeyPublic {
        id: KID.to_string(),
        algorithm: "ES256".to_string(),
        public_key: key.public_pem.clone(),
        active: true,
        created_at: Utc::now(),
    };
    serde_json::to_value(build_jwks(&[row]).expect("JWKS")).expect("JWKS JSON")
}

struct Harness {
    app: axum::Router,
    jwks: Value,
    minter: JwtMinter,
    _key_file: tempfile::NamedTempFile,
    _mocks: MockServer,
}

async fn harness(db: Option<Arc<Database>>) -> Harness {
    let key = es256_key();
    let mut key_file = tempfile::NamedTempFile::new().expect("key file");
    key_file
        .write_all(key.private_pem.as_bytes())
        .expect("write key");
    let jwt = JwtConfig {
        signing_algorithm: "ES256".to_string(),
        signing_key_path: Some(key_file.path().to_string_lossy().into_owned()),
        signing_key_secret: None,
        signing_key_id: Some(KID.to_string()),
        issuer: ISSUER.to_string(),
        default_ttl_seconds: 300,
    };
    let jwks = gate_jwks(&key);

    // One mock server plays Kratos and the inbound-JWT issuer (gate itself).
    let mocks = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sessions/whoami"))
        .and(header_regex("cookie", "ory_kratos_session=valid-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "session-1",
            "active": true,
            "expires_at": "2100-01-01T00:00:00Z",
            "identity": {"id": KRATOS_USER, "traits": {"email": "visitor@know-me.tools"}}
        })))
        .mount(&mocks)
        .await;
    Mock::given(method("GET"))
        .and(path("/sessions/whoami"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mocks)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks.clone()))
        .mount(&mocks)
        .await;

    let yaml = format!(
        r#"
auth_providers:
  kratos: {{ type: kratos, base_url: "{base}" }}
  bearer: {{ type: jwt, jwks_url: "{base}/jwks.json", issuer: "{ISSUER}" }}
  public: {{ type: anonymous }}
  keys: {{ type: api_key }}
sites:
  - id: site
    domains: ["{HOST}"]
routes:
  - id: root
    site: site
    match: {{ path: "/" }}
    auth: public
  - id: public
    site: site
    match: {{ path: "/public/**" }}
    auth: public
    hooks:
      pre_request:
        - type: claims_enhancement
          config:
            mint_jwt:
              enabled: true
              additional_claims: {{ aud: "uar", via: "{{{{ identity.id }}}}" }}
  - id: session
    site: site
    match: {{ path: "/kratos/**" }}
    auth: kratos
  - id: service
    site: site
    match: {{ path: "/jwt/**" }}
    auth: bearer
  - id: keyed
    site: site
    match: {{ path: "/keys/**" }}
    auth: keys
  - id: transformed
    site: site
    match: {{ path: "/transformed/**" }}
    auth: public
    hooks:
      pre_request:
        - type: body_transform
          config: {{ set_fields: {{ user: "{{{{ identity.id }}}}" }} }}
"#,
        base = mocks.uri()
    );
    let mut config: GateConfig = serde_yaml::from_str(&yaml).expect("config");
    config.jwt = jwt.clone();

    let http_client = reqwest::Client::new();
    let providers = build_authenticators(&config.auth_providers, &http_client, db.clone());
    let minter = JwtMinter::from_db_or_config(db.as_deref(), &jwt)
        .await
        .expect("minter");
    let state = Arc::new(AppState {
        router: Arc::new(RwLock::new(GateRouter::from_config(&config))),
        config: Arc::new(RwLock::new(config)),
        auth_providers: Arc::new(providers),
        jwt_minter: Arc::new(RwLock::new(Some(minter.clone()))),
        cache: Arc::new(GateCache::from_config(&Default::default())),
        db: db.clone(),
        http_client,
        lookup_registry: Arc::new(LookupRegistry::new(db)),
        authz: Arc::new(AuthzEngine::empty()),
        approval_manager: ApprovalManager::new(),
        approval_store: MemoryApprovalStore::new(),
        #[cfg(feature = "redis-l2")]
        rate_limiter: None,
    });

    let mut app = axum::Router::new();
    for route in EXT_AUTHZ_ROUTES {
        app = app.route(route, axum::routing::any(ext_authz_handler));
    }
    Harness {
        app: app.with_state(state),
        jwks,
        minter,
        _key_file: key_file,
        _mocks: mocks,
    }
}

/// An Envoy-shaped check request for a client request `method original_path`.
fn check(method: &str, original_path: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/ext-authz{original_path}"))
        .header(header::HOST, HOST)
        .header(header::CONTENT_LENGTH, "0");
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    req.body(Body::empty()).expect("request")
}

async fn send(h: &Harness, req: Request<Body>) -> axum::response::Response {
    h.app.clone().oneshot(req).await.expect("response")
}

/// Verify the injected bearer against `jwks` (gate's published key set).
fn verify_bearer(resp: &axum::response::Response, jwks: &Value) -> Value {
    let auth = resp
        .headers()
        .get(header::AUTHORIZATION)
        .expect("injected Authorization")
        .to_str()
        .expect("ascii");
    let token = auth.strip_prefix("Bearer ").expect("Bearer scheme");
    let kid = decode_header(token).expect("JWT header").kid.expect("kid");
    let set: JwkSet = serde_json::from_value(jwks.clone()).expect("standard JWKS");
    let key = DecodingKey::from_jwk(set.find(&kid).expect("kid in JWKS")).expect("EC key");
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_issuer(&[ISSUER]);
    validation.validate_aud = false;
    decode::<Value>(token, &key, &validation)
        .expect("bearer verifies against gate JWKS")
        .claims
}

fn assert_denied_without_token(resp: &axum::response::Response, status: StatusCode) {
    assert_eq!(resp.status(), status);
    assert!(resp.headers().get(header::AUTHORIZATION).is_none());
    assert!(resp.headers().get(ENVOY_HEADERS_TO_REMOVE).is_none());
}

/// Envoy (`headersToBackend: [authorization]`) sets each returned
/// `Authorization` on the upstream request and removes the headers listed in
/// `x-envoy-auth-headers-to-remove`. Apply the same rules to the client's
/// headers to get what the upstream receives.
fn upstream_headers(
    client: &[(&str, &str)],
    resp: &axum::response::Response,
) -> Vec<(String, String)> {
    let remove: Vec<String> = resp
        .headers()
        .get(ENVOY_HEADERS_TO_REMOVE)
        .map(|v| {
            v.to_str()
                .unwrap()
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<(String, String)> = client
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
        .filter(|(k, _)| k != "authorization" && !remove.contains(k))
        .collect();
    if let Some(auth) = resp.headers().get(header::AUTHORIZATION) {
        out.push(("authorization".into(), auth.to_str().unwrap().into()));
    }
    out
}

#[tokio::test]
async fn anonymous_route_allows_with_a_gate_bearer_and_drops_client_credentials() {
    let h = harness(None).await;
    let client = [
        ("authorization", "Bearer client-supplied-token"),
        ("x-api-key", "client-supplied-key"),
        ("accept", "text/html"),
    ];
    let resp = send(&h, check("GET", "/public/pricing?ref=nav", &client)).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let claims = verify_bearer(&resp, &h.jwks);
    assert_eq!(claims["sub"], "anonymous");
    assert_eq!(claims["aud"], "uar");
    assert_eq!(
        claims["via"], "anonymous",
        "route mint_jwt templates render"
    );

    let upstream = upstream_headers(&client, &resp);
    let auth: Vec<_> = upstream
        .iter()
        .filter(|(k, _)| k == "authorization")
        .collect();
    assert_eq!(auth.len(), 1);
    assert_ne!(auth[0].1, "Bearer client-supplied-token");
    assert!(upstream.iter().all(|(k, _)| k != "x-api-key"));
    assert!(upstream.iter().all(|(_, v)| !v.contains("client-supplied")));
    assert!(upstream.contains(&("accept".into(), "text/html".into())));
    let removed = resp.headers()[ENVOY_HEADERS_TO_REMOVE].to_str().unwrap();
    assert!(
        !removed.contains("authorization"),
        "Envoy removes after it sets, so removing authorization would drop the gate bearer"
    );
}

#[tokio::test]
async fn root_path_check_resolves_the_root_route() {
    let h = harness(None).await;
    let resp = send(&h, check("GET", "/", &[])).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(verify_bearer(&resp, &h.jwks)["sub"], "anonymous");
}

#[tokio::test]
async fn kratos_session_allows_as_the_session_identity() {
    let h = harness(None).await;
    let resp = send(
        &h,
        check(
            "POST",
            "/kratos/api/chat",
            &[("cookie", "ory_kratos_session=valid-session")],
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let claims = verify_bearer(&resp, &h.jwks);
    assert_eq!(claims["sub"], KRATOS_USER);
    assert_eq!(claims["email"], "visitor@know-me.tools");
}

#[tokio::test]
async fn kratos_route_denies_without_a_valid_session() {
    let h = harness(None).await;
    let none = send(&h, check("GET", "/kratos/api/chat", &[])).await;
    assert_denied_without_token(&none, StatusCode::UNAUTHORIZED);
    let bad = send(
        &h,
        check(
            "GET",
            "/kratos/api/chat",
            &[("cookie", "ory_kratos_session=expired")],
        ),
    )
    .await;
    assert_denied_without_token(&bad, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_route_allows_a_verified_bearer_and_reissues_it() {
    let h = harness(None).await;
    let presented = h
        .minter
        .mint(
            &Identity {
                id: "svc-site".to_string(),
                ..Default::default()
            },
            None,
            None,
        )
        .expect("presented token");
    let auth = format!("Bearer {presented}");
    let resp = send(
        &h,
        check("GET", "/jwt/v1/runs", &[("authorization", &auth)]),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(verify_bearer(&resp, &h.jwks)["sub"], "svc-site");
    assert_ne!(
        resp.headers()[header::AUTHORIZATION].to_str().unwrap(),
        auth
    );
}

#[tokio::test]
async fn jwt_route_denies_missing_or_forged_bearer() {
    let h = harness(None).await;
    let none = send(&h, check("GET", "/jwt/v1/runs", &[])).await;
    assert_denied_without_token(&none, StatusCode::UNAUTHORIZED);
    let forged = send(
        &h,
        check(
            "GET",
            "/jwt/v1/runs",
            &[("authorization", "Bearer eyJhbGciOiJIUzI1NiJ9.e30.c2ln")],
        ),
    )
    .await;
    assert_denied_without_token(&forged, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_key_route_without_a_database_fails_closed() {
    let h = harness(None).await;
    let resp = send(&h, check("GET", "/keys/data", &[("x-api-key", "any-key")])).await;
    assert_denied_without_token(&resp, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn unmatched_host_and_unsupported_hooks_are_denied() {
    let h = harness(None).await;
    let mut other_host = check("GET", "/public/pricing", &[]);
    other_host
        .headers_mut()
        .insert(header::HOST, "evil.example".parse().unwrap());
    assert_denied_without_token(&send(&h, other_host).await, StatusCode::FORBIDDEN);

    let hooked = send(&h, check("POST", "/transformed/x", &[])).await;
    assert_denied_without_token(&hooked, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires a live Postgres via DATABASE_URL"]
async fn api_key_route_allows_and_bearer_verifies_against_served_jwks() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for this test");
    let db = Arc::new(Database::connect(&url, 2).await.expect("connect"));
    db.migrate().await.expect("migrate");
    // Seed gate's signing key in the database, as production does, so the
    // minter and the served JWKS both come from the real DB path.
    let key = es256_key();
    let kid = format!("ext-authz-test-{}", uuid::Uuid::new_v4());
    db.insert_signing_key(&kid, "ES256", &key.public_pem, &key.private_pem)
        .await
        .expect("seed signing key");
    let client_id = format!("ext-authz-test-{}", uuid::Uuid::new_v4());
    let (_, raw_key) = db
        .create_api_key(&client_id, &["chat".to_string()], None)
        .await
        .expect("api key");

    let h = harness(Some(Arc::clone(&db))).await;
    let served = axum::response::IntoResponse::into_response(jwks_handler(Some(db)).await);
    let body = axum::body::to_bytes(served.into_body(), 1 << 20)
        .await
        .expect("JWKS body");
    let served_jwks: Value = serde_json::from_slice(&body).expect("JWKS JSON");

    let client = [
        ("x-api-key", raw_key.as_str()),
        ("authorization", "Bearer client-supplied-token"),
    ];
    let resp = send(&h, check("POST", "/keys/data", &client)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let claims = verify_bearer(&resp, &served_jwks);
    assert_eq!(claims["sub"], client_id);
    let upstream = upstream_headers(&client, &resp);
    assert!(upstream.iter().all(|(k, _)| k != "x-api-key"));
    assert!(upstream.iter().all(|(_, v)| !v.contains("client-supplied")));
    assert!(upstream.iter().all(|(_, v)| !v.contains(&raw_key)));
}
