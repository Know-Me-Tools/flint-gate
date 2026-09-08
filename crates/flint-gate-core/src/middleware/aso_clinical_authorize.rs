//! Fresh ASO clinical authority check, independent of Gate's identity cache.
use crate::config::types::AsoClinicalAuthorizeConfig;
use http::{HeaderMap, StatusCode};
use std::{sync::LazyLock, time::Duration};

const CREDENTIAL_HEADERS: [&str; 3] = ["cookie", "authorization", "x-session-token"];
static AUTHORIZATION_CLIENT: LazyLock<Result<reqwest::Client, ()>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| ())
});

/// Callback decisions have no body or identity claims to trust or forward.
pub(super) async fn authorize(
    config: &AsoClinicalAuthorizeConfig,
    method: &str,
    uri: &str,
    original_headers: &HeaderMap,
) -> Result<(), StatusCode> {
    let credentials = credential_headers(original_headers)?;
    let url = callback_url(&config.url)?;
    // Reuse a dedicated client: the ordinary upstream client may permit
    // redirects or proxies, which could disclose a session credential.
    let client = AUTHORIZATION_CLIENT
        .as_ref()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = client
        .post(url)
        .headers(credentials)
        .json(&serde_json::json!({ "method": method, "uri": uri }))
        .send()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // Do not read, log, or propagate callback bodies or headers. Only an exact
    // 204 can authorize the clinical request; all other successes fail closed.
    match response.status() {
        StatusCode::NO_CONTENT => Ok(()),
        StatusCode::UNAUTHORIZED => Err(StatusCode::UNAUTHORIZED),
        StatusCode::FORBIDDEN => Err(StatusCode::FORBIDDEN),
        _ => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

pub(super) fn callback_url(raw: &str) -> Result<reqwest::Url, StatusCode> {
    let url = reqwest::Url::parse(raw).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(url)
}

pub(super) fn credential_headers(original: &HeaderMap) -> Result<HeaderMap, StatusCode> {
    let mut credentials = HeaderMap::new();
    for name in CREDENTIAL_HEADERS {
        let mut values = original.get_all(name).iter();
        if let Some(value) = values.next() {
            // Reject ambiguity before upstream forwarding can collapse repeated
            // headers. ASO decides whether the one raw credential is valid.
            if values.next().is_some()
                || !credentials.is_empty()
                || value.as_bytes().iter().all(u8::is_ascii_whitespace)
            {
                return Err(StatusCode::UNAUTHORIZED);
            }
            credentials.append(name, value.clone());
        }
    }
    if credentials.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

    fn config(server: &MockServer) -> AsoClinicalAuthorizeConfig {
        AsoClinicalAuthorizeConfig {
            url: format!("{}/internal/gate/authorize", server.uri()),
        }
    }

    fn headers(name: &'static str, value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.append(name, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn parses_named_hook_and_rejects_unknown_options() {
        let hook: crate::config::types::PreRequestHook = serde_yaml::from_str(
            "type: aso_clinical_authorize\nconfig:\n  url: http://aso:8080/internal/gate/authorize\n",
        )
        .unwrap();
        assert!(matches!(
            hook,
            crate::config::types::PreRequestHook::AsoClinicalAuthorize { .. }
        ));
        assert!(serde_yaml::from_str::<AsoClinicalAuthorizeConfig>(
            "url: http://aso:8080/internal/gate/authorize\nenforce: false\n"
        )
        .is_err());
    }

    #[test]
    fn only_fixed_http_endpoints_without_extra_authority_are_valid() {
        for invalid in [
            "/internal/gate/authorize",
            "file:///internal/gate/authorize",
            "http://user@aso/authorize",
            "http://user:secret@aso/authorize",
            "http://aso/authorize?mode=allow",
            "http://aso/authorize?",
            "http://aso/authorize#fragment",
        ] {
            assert_eq!(callback_url(invalid), Err(StatusCode::SERVICE_UNAVAILABLE));
        }
        assert!(callback_url("http://host.docker.internal:8080/internal/gate/authorize").is_ok());
        assert!(callback_url("https://aso.example/internal/gate/authorize").is_ok());
    }

    #[tokio::test]
    async fn forwards_original_metadata_and_only_raw_credentials_on_every_call() {
        let server = MockServer::start().await;
        let uri = "/api/cases/case%2Fid/affirm?revision=2&mode=a%20b";
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/internal/gate/authorize"))
            .and(matchers::body_json(
                serde_json::json!({"method": "PATCH", "uri": uri}),
            ))
            .respond_with(ResponseTemplate::new(204))
            .expect(6)
            .mount(&server)
            .await;
        for (name, value) in [
            ("cookie", "ory_kratos_session=synthetic; theme=light"),
            ("authorization", "Bearer synthetic"),
            ("x-session-token", "synthetic-session"),
        ] {
            let mut original = headers(name, value);
            original.insert("x-flint-role", HeaderValue::from_static("surgeon"));
            original.insert("x-aso-principal", HeaderValue::from_static("forged"));
            for _ in 0..2 {
                assert_eq!(
                    authorize(&config(&server), "PATCH", uri, &original).await,
                    Ok(())
                );
            }
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 6);
        for (index, request) in requests.iter().enumerate() {
            let (name, value) = [
                ("cookie", "ory_kratos_session=synthetic; theme=light"),
                ("authorization", "Bearer synthetic"),
                ("x-session-token", "synthetic-session"),
            ][index / 2];
            assert_eq!(request.headers.get_all(name).iter().count(), 1);
            assert_eq!(
                request.headers.get(name).unwrap().as_bytes(),
                value.as_bytes()
            );
            assert!(!request.headers.contains_key("x-flint-role"));
            assert!(!request.headers.contains_key("x-aso-principal"));
        }
    }

    #[tokio::test]
    async fn rejects_missing_duplicate_and_mixed_credentials_without_network() {
        let server = MockServer::start().await;
        let mut cases = vec![HeaderMap::new()];
        for name in CREDENTIAL_HEADERS {
            cases.push(headers(name, " "));
            let mut duplicate = headers(name, "synthetic");
            duplicate.append(name, HeaderValue::from_static("other"));
            cases.push(duplicate);
            for other in CREDENTIAL_HEADERS {
                if other != name {
                    let mut mixed = headers(name, "synthetic");
                    mixed.append(other, HeaderValue::from_static("other"));
                    cases.push(mixed);
                }
            }
        }
        for original in cases {
            assert_eq!(
                authorize(&config(&server), "POST", "/api/cases/x/affirm", &original).await,
                Err(StatusCode::UNAUTHORIZED)
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn preserves_only_denial_status_and_requires_exact_204() {
        for status in [200, 201, 202, 401, 403, 404, 500, 503] {
            let server = MockServer::start().await;
            Mock::given(matchers::method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_string("untrusted details"))
                .expect(1)
                .mount(&server)
                .await;
            let expected = match status {
                401 => StatusCode::UNAUTHORIZED,
                403 => StatusCode::FORBIDDEN,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            assert_eq!(
                authorize(
                    &config(&server),
                    "POST",
                    "/api/cases/x/affirm",
                    &headers("cookie", "synthetic")
                )
                .await,
                Err(expected)
            );
        }
    }

    #[tokio::test]
    async fn refuses_redirects_without_disclosing_credentials() {
        let destination = MockServer::start().await;
        let source = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", destination.uri()))
            .expect(1)
            .mount(&source)
            .await;
        assert_eq!(
            authorize(
                &config(&source),
                "POST",
                "/api/cases/x/affirm",
                &headers("cookie", "synthetic")
            )
            .await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert!(destination.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn slow_callback_times_out_without_retry() {
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_secs(30)))
            .expect(1)
            .mount(&server)
            .await;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            authorize(
                &config(&server),
                "POST",
                "/api/cases/x/affirm",
                &headers("cookie", "synthetic"),
            ),
        )
        .await;
        assert_eq!(result.unwrap(), Err(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn unavailable_callback_fails_closed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let config = AsoClinicalAuthorizeConfig {
            url: format!("http://{address}/internal/gate/authorize"),
        };
        assert_eq!(
            authorize(
                &config,
                "POST",
                "/api/cases/x/affirm",
                &headers("cookie", "synthetic")
            )
            .await,
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }
}
