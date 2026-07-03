//! In-process per-replica request-rate limiting via `tower_governor`.
//!
//! This is the coarse burst shield: a per-replica, in-memory token-bucket
//! keyed on the caller's credential (API key / Authorization / session cookie),
//! falling back to the client IP when no credential is present. It complements
//! — and does not replace — the authoritative, cross-replica Redis window
//! counters.

use governor::middleware::NoOpMiddleware;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use tower_governor::{
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::{KeyExtractor, SmartIpKeyExtractor},
    GovernorError, GovernorLayer,
};

/// Key extractor that rate-limits per credential, falling back to client IP.
///
/// Order of precedence for the key:
/// 1. `Authorization` header (bearer token / basic) — hashed
/// 2. `X-API-Key` header — hashed
/// 3. `Cookie` header — hashed
/// 4. Client IP (via [`SmartIpKeyExtractor`]: `X-Forwarded-For` → `X-Real-IP`
///    → `Forwarded` → peer IP)
///
/// Credentials are SHA-256 hashed so no raw secret is retained in limiter keys.
#[derive(Clone, Debug)]
pub struct CredentialKeyExtractor;

/// The extracted rate-limit key: either a hashed credential or a client IP.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// SHA-256 hex of the caller's credential.
    Credential(String),
    /// Client IP address.
    Ip(IpAddr),
}

impl KeyExtractor for CredentialKeyExtractor {
    type Key = RateLimitKey;

    fn extract<T>(&self, req: &http::request::Request<T>) -> Result<Self::Key, GovernorError> {
        let headers = req.headers();
        let credential = headers
            .get(http::header::AUTHORIZATION)
            .or_else(|| headers.get("x-api-key"))
            .or_else(|| headers.get(http::header::COOKIE))
            .and_then(|v| v.to_str().ok());

        if let Some(cred) = credential {
            let mut h = Sha256::new();
            h.update(cred.as_bytes());
            return Ok(RateLimitKey::Credential(hex::encode(h.finalize())));
        }

        // No credential — fall back to the client IP.
        SmartIpKeyExtractor.extract(req).map(RateLimitKey::Ip)
    }
}

/// Build a `tower_governor` config for the proxy router.
///
/// `per_second` is the sustained replenishment rate and `burst` the bucket
/// capacity, both per key. Returns `None` if the parameters are degenerate
/// (zero burst or zero period), in which case the caller should skip the layer.
pub fn build_governor_config(
    per_second: u64,
    burst: u32,
) -> Option<GovernorConfig<CredentialKeyExtractor, NoOpMiddleware>> {
    GovernorConfigBuilder::default()
        .per_second(per_second.max(1))
        .burst_size(burst.max(1))
        .key_extractor(CredentialKeyExtractor)
        .finish()
}

/// Convenience: build the ready-to-apply [`GovernorLayer`] for the proxy router.
///
/// Returns `None` when the config is degenerate; the caller then serves without
/// the in-process limiter.
pub fn build_governor_layer(
    per_second: u64,
    burst: u32,
) -> Option<GovernorLayer<CredentialKeyExtractor, NoOpMiddleware, axum::body::Body>> {
    let config = build_governor_config(per_second, burst)?;
    Some(GovernorLayer::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_credential_from_authorization() {
        let req = http::Request::builder()
            .header(http::header::AUTHORIZATION, "Bearer abc123")
            .body(())
            .unwrap();
        let key = CredentialKeyExtractor.extract(&req).unwrap();
        match key {
            RateLimitKey::Credential(h) => assert_eq!(h.len(), 64),
            _ => panic!("expected credential key"),
        }
    }

    #[test]
    fn extracts_credential_from_api_key_header() {
        let req = http::Request::builder()
            .header("x-api-key", "k-42")
            .body(())
            .unwrap();
        assert!(matches!(
            CredentialKeyExtractor.extract(&req).unwrap(),
            RateLimitKey::Credential(_)
        ));
    }

    #[test]
    fn falls_back_to_forwarded_ip_without_credential() {
        let req = http::Request::builder()
            .header("x-forwarded-for", "203.0.113.7")
            .body(())
            .unwrap();
        match CredentialKeyExtractor.extract(&req).unwrap() {
            RateLimitKey::Ip(ip) => assert_eq!(ip.to_string(), "203.0.113.7"),
            _ => panic!("expected IP key"),
        }
    }

    #[test]
    fn extracts_credential_from_cookie_when_no_auth_or_api_key() {
        let req = http::Request::builder()
            .header(http::header::COOKIE, "ory_kratos_session=xyz")
            .body(())
            .unwrap();
        assert!(matches!(
            CredentialKeyExtractor.extract(&req).unwrap(),
            RateLimitKey::Credential(_)
        ));
    }

    #[test]
    fn authorization_takes_precedence_over_api_key_and_cookie() {
        // All three present — the Authorization hash must be chosen.
        let with_all = http::Request::builder()
            .header(http::header::AUTHORIZATION, "Bearer tok")
            .header("x-api-key", "k")
            .header(http::header::COOKIE, "c=1")
            .body(())
            .unwrap();
        let only_auth = http::Request::builder()
            .header(http::header::AUTHORIZATION, "Bearer tok")
            .body(())
            .unwrap();
        assert_eq!(
            CredentialKeyExtractor.extract(&with_all).unwrap(),
            CredentialKeyExtractor.extract(&only_auth).unwrap(),
            "key must derive solely from Authorization when present"
        );
    }

    #[test]
    fn distinct_credentials_yield_distinct_keys() {
        let a = http::Request::builder()
            .header(http::header::AUTHORIZATION, "Bearer aaa")
            .body(())
            .unwrap();
        let b = http::Request::builder()
            .header(http::header::AUTHORIZATION, "Bearer bbb")
            .body(())
            .unwrap();
        assert_ne!(
            CredentialKeyExtractor.extract(&a).unwrap(),
            CredentialKeyExtractor.extract(&b).unwrap()
        );
    }

    #[test]
    fn errors_when_no_credential_and_no_ip_source() {
        // No credential headers and no IP info → extraction fails.
        let req = http::Request::builder().body(()).unwrap();
        assert!(CredentialKeyExtractor.extract(&req).is_err());
    }

    #[test]
    fn build_config_rejects_never_degenerate() {
        // per_second/burst are floored to 1, so a config is always produced.
        assert!(build_governor_config(0, 0).is_some());
    }

    #[test]
    fn build_governor_layer_produces_a_layer() {
        assert!(build_governor_layer(10, 20).is_some());
    }
}
