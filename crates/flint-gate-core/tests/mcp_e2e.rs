//! Task 8 — end-to-end MCP OAuth 2.1 Resource Server handshake.
//!
//! Drives the `McpAuthenticator` against a mock Authorization Server (wiremock,
//! in-process — no live infra, so these run in the default suite) proving:
//!   (a) a valid token with correct `aud` + scopes → authorized `Identity`;
//!   (b) a token with the wrong `aud` → 401 rejected (RFC 8707 confused-deputy);
//!   (c) a token missing a required scope → `InsufficientScope` (403 step-up);
//!   (d) the RFC 9728 `.well-known/oauth-protected-resource` JSON is correct.
//!
//! The signing key is a fixed 2048-bit RSA keypair; the mock AS serves its
//! public half as a JWKS so signature verification exercises the real path.

use flint_gate_core::auth::mcp::McpAuthenticator;
use flint_gate_core::auth::mcp_metadata::protected_resource_metadata;
use flint_gate_core::auth::{AuthError, AuthMethod, Authenticator};
use flint_gate_core::config::types::McpAuthConfig;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KID: &str = "test-key-1";
const RESOURCE: &str = "https://gate.example/mcp";
const ISSUER: &str = "https://as.example";

/// Fixed test RSA private key (PKCS#8 PEM).
const RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDLHJK37aHT49E5
Zd5C9wPsWLMXvfUUswMYHmqgRXACiF2efAQIgoudhl/e/ZrDU1ZZBWvmeFR9KI3Q
38s54Y/MMcXzOHZvYNzbEaCyD7yeeUrxpryQ4qU+ILU4jxRu2RFGbkpEx+57JTuY
wPa5d8R6/inmG0D/7U3E0afci5QcceTIhRQMAcawdW7e8Ox94L3yWPttLUmMMZaH
LjbTW+d3KhYVsZS04/dclXScTvfaCn7845WeO+JUgcfbxh+8EKMNGWMO20fUH/sj
WmTK18aakflyuUcm4adqexjm68bX5RC08Lqz5zsI14oU/+LuyEs8nSxREmMPZdJp
C9vva2lJAgMBAAECggEAFLJGqQNH2CWArk6ZBU3SUoT/Ss4KaR+MkyqWRtqMfVWT
5JBSgON8goKsxjYlTP3y8INC6WsdgNtfCAel17IKSw5PU2dPei3uk0wKidpcp/FI
F7Obx0+w8tG2ZZr+xATOY9TdMIEG5fl3RytyeJehp766Wj4ws2Nk50dNyYVXhEZQ
Q4GPL4FiC8v/AmemsLpi3gJ6gQ/3v7aM6bLwiPfYy2V3Xy4oSwvkw8PDh+Krtkus
Z/DKhNW4TxujvxIRXj0xMFJQYGASph64DxRdHVl6nAya9nfTA0zksQAjt7nKLOCR
yZa/O5+C6ALsgT1eNqUyVi5pa55/LLE1VNSxwPlXuQKBgQDpsP5fvxJYgvXeGMKC
lJRfooBPIfxUw7XME9tvL9UoG9V3/IaIIIU70DfIFs7I6g3HZ0EEydxGUkZ0s10h
tqCynMjTllCkXxszCirmqXqoXZI3Ql7rBnEsjTs67JR/w0iDiIkSykh2dwxJxxqa
eyFd2J/vA26m9F2Nuqma+In9hwKBgQDegEOD4ZNxqMgQaewhD4Pki5jdOf/How6q
eDxiIbXBTXF+Gi7Tfb7Ok1KVdRXvQ7vnhJGHB7ulVKdR08YhDCmnsqcrntvyHN9v
yFwHw2uv2iATCMLhaYJYIxBN67dFmSUTG/oj34xFKa7dFmRE2Qgu94y0I9uYtx33
A2jTL6aWrwKBgQDdl7edktESnRwHPfMzXzBSfwSsBM4AkpQQr8Oj6vd00O/altn6
utubnBVI5leurEHkk0RUBhWZmOq2Pl5RWZuHwqOr/xz4wDZKb5m+n3ZvsEq1m3nl
4nXuiP1hInStsb9Q+mcAKlAMBVbhnqrbUWaSVpdRTS/foFgVzKqHCKXQgQKBgQC5
6oD/rLhQC5EILgmxYk555K9Zg0IXpUb26DrEYJiHqddAYE5qR7Ls16r02X33jChx
fpM/OhXwQvkAZa0zJf+UcbI/v6DXAIsu00Ma9Y6AxQlx/isgwNG6JapVAbYFAL86
5XCxEvUZQYgskq473QF6hTzbtO6j/7aZFQ89D57qXwKBgDNDrAlWpCkf0Ba9oyXd
P2VCJeI4J+FCYXskq0OM2dg1CKmeLlTRkkNPmxKRG3xOy+n2FmwGkq8YlfTFgDQu
KqCMU1qViU4Kn+fZqRpEr/43bqgiBQgzUCGJ4WsIu0MKbvqWXKDG036wJuQc88lI
mO87a5mPVQ24t8NflryuA76H
-----END PRIVATE KEY-----";

/// The matching RSA public modulus/exponent as JWK base64url params.
const JWK_N: &str = "yxySt-2h0-PROWXeQvcD7FizF731FLMDGB5qoEVwAohdnnwECIKLnYZf3v2aw1NWWQVr5nhUfSiN0N_LOeGPzDHF8zh2b2Dc2xGgsg-8nnlK8aa8kOKlPiC1OI8UbtkRRm5KRMfueyU7mMD2uXfEev4p5htA_-1NxNGn3IuUHHHkyIUUDAHGsHVu3vDsfeC98lj7bS1JjDGWhy4201vndyoWFbGUtOP3XJV0nE732gp-_OOVnjviVIHH28YfvBCjDRljDttH1B_7I1pkytfGmpH5crlHJuGnansY5uvG1-UQtPC6s-c7CNeKFP_i7shLPJ0sURJjD2XSaQvb72tpSQ";
const JWK_E: &str = "AQAB";

#[derive(serde::Serialize)]
struct TokenClaims {
    sub: String,
    iss: String,
    aud: String,
    exp: usize,
    scope: String,
}

/// Sign a token with the fixed key + `KID`, with the given audience and scopes.
fn mint_token(aud: &str, scope: &str) -> String {
    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(KID.to_string());
    let claims = TokenClaims {
        sub: "agent-007".to_string(),
        iss: ISSUER.to_string(),
        aud: aud.to_string(),
        // Far-future expiry so the test isn't time-sensitive.
        exp: 4_102_444_800, // 2100-01-01
        scope: scope.to_string(),
    };
    let key = EncodingKey::from_rsa_pem(RSA_PRIVATE_PEM.as_bytes()).expect("valid RSA PEM");
    encode(&header, &claims, &key).expect("token encodes")
}

/// Start a mock AS serving the JWKS at `/jwks`, returning its base URL.
async fn start_mock_as() -> MockServer {
    let server = MockServer::start().await;
    let jwks = json!({
        "keys": [
            { "kty": "RSA", "kid": KID, "use": "sig", "alg": "RS256", "n": JWK_N, "e": JWK_E }
        ]
    });
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
        .mount(&server)
        .await;
    server
}

fn mcp_config(jwks_url: String, required_scopes: Vec<String>) -> McpAuthConfig {
    McpAuthConfig {
        jwks_url,
        issuer: Some(ISSUER.to_string()),
        audience: Some(RESOURCE.to_string()),
        resource: RESOURCE.to_string(),
        authorization_servers: vec![ISSUER.to_string()],
        required_scopes,
        leeway_seconds: 5,
    }
}

fn parts_with_bearer(token: &str) -> http::request::Parts {
    let (mut parts, _) = http::Request::new(()).into_parts();
    parts.headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    parts
}

// (a) Valid token with correct aud + scopes → authorized Identity.
#[tokio::test]
async fn valid_token_authorizes_with_identity_and_scopes() {
    let server = start_mock_as().await;
    let cfg = mcp_config(format!("{}/jwks", server.uri()), vec!["mcp:read".to_string()]);
    let auth = McpAuthenticator::new(cfg, reqwest::Client::new());

    let token = mint_token(RESOURCE, "mcp:read mcp:write");
    let result = auth
        .authenticate(&parts_with_bearer(&token))
        .await
        .expect("valid token authorizes");

    assert_eq!(result.identity.id, "agent-007");
    match result.method {
        AuthMethod::McpBearer { scopes } => {
            assert!(scopes.contains(&"mcp:read".to_string()));
            assert!(scopes.contains(&"mcp:write".to_string()));
        }
        other => panic!("expected McpBearer, got {other:?}"),
    }
}

// (b) Wrong audience → 401 (RFC 8707 confused-deputy defense).
#[tokio::test]
async fn wrong_audience_is_rejected() {
    let server = start_mock_as().await;
    let cfg = mcp_config(format!("{}/jwks", server.uri()), vec![]);
    let auth = McpAuthenticator::new(cfg, reqwest::Client::new());

    // Token minted for a DIFFERENT resource — signature is valid, aud is not.
    let token = mint_token("https://someone-else.example/mcp", "mcp:read");
    let result = auth.authenticate(&parts_with_bearer(&token)).await;
    match result {
        Err(AuthError::Unauthorized(msg)) => {
            assert!(msg.contains("audience"), "expected aud rejection, got: {msg}");
        }
        other => panic!("expected Unauthorized (RFC 8707), got {other:?}"),
    }
}

// (c) Missing required scope → InsufficientScope (403 step-up).
#[tokio::test]
async fn missing_scope_is_insufficient_scope() {
    let server = start_mock_as().await;
    let cfg = mcp_config(
        format!("{}/jwks", server.uri()),
        vec!["mcp:admin".to_string()],
    );
    let auth = McpAuthenticator::new(cfg, reqwest::Client::new());

    // Token has read/write but NOT the required mcp:admin.
    let token = mint_token(RESOURCE, "mcp:read mcp:write");
    let result = auth.authenticate(&parts_with_bearer(&token)).await;
    match result {
        Err(AuthError::InsufficientScope { required }) => {
            assert_eq!(required, vec!["mcp:admin".to_string()]);
        }
        other => panic!("expected InsufficientScope, got {other:?}"),
    }
}

// (d) RFC 9728 Protected Resource Metadata JSON shape.
#[test]
fn protected_resource_metadata_is_rfc9728() {
    let cfg = mcp_config(
        "https://as.example/jwks".to_string(),
        vec!["mcp:read".to_string(), "mcp:write".to_string()],
    );
    let m = protected_resource_metadata(&cfg);
    assert_eq!(m["resource"], RESOURCE);
    assert_eq!(m["authorization_servers"][0], ISSUER);
    assert_eq!(m["scopes_supported"], json!(["mcp:read", "mcp:write"]));
    assert_eq!(m["bearer_methods_supported"], json!(["header"]));
}

// Bonus: a tampered-signature token (valid structure, wrong key) is rejected,
// proving the mock JWKS signature check is actually enforced end-to-end.
#[tokio::test]
async fn tampered_signature_is_rejected() {
    let server = start_mock_as().await;
    let cfg = mcp_config(format!("{}/jwks", server.uri()), vec![]);
    let auth = McpAuthenticator::new(cfg, reqwest::Client::new());

    let mut token = mint_token(RESOURCE, "mcp:read");
    // Flip the last char of the signature segment.
    let last = token.pop().unwrap();
    token.push(if last == 'A' { 'B' } else { 'A' });

    let result = auth.authenticate(&parts_with_bearer(&token)).await;
    assert!(matches!(result, Err(AuthError::Unauthorized(_))));
}
