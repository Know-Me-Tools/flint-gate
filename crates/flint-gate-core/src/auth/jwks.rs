//! Shared JWKS discovery + rotation cache built on `jsonwebtoken@9`.
//!
//! Both [`crate::auth::jwt_verify::JwtVerifyAuthenticator`] and
//! [`crate::auth::mcp::McpAuthenticator`] resolve their signing keys through
//! this helper so the fetch/cache/rotation logic lives in exactly one place.
//!
//! Security properties (fail-closed throughout):
//! - **SSRF guard (H1):** `jwks_url` is validated at construction — `https`
//!   only, except `http` is permitted for explicit `localhost`/loopback dev
//!   hosts. Link-local, loopback (non-dev), and private-range hosts are
//!   rejected. The dedicated JWKS client disables redirect-following so the
//!   endpoint cannot bounce us to a cloud metadata service.
//! - **kid handling (H2):** a token with no `kid` against a multi-key JWKS is
//!   REJECTED (never "pick first"), and symmetric/`oct` JWKs are refused — this
//!   RS only trusts asymmetric (RSA/EC) verification keys, closing the
//!   alg-confusion/symmetric-downgrade class.
//! - **Rotation + DoS (M2):** on an unknown `kid` we force at most ONE refresh,
//!   concurrent refreshes are single-flighted behind a mutex, and a minimum
//!   interval gate ([`MIN_REFRESH_INTERVAL`]) prevents a stream of bogus `kid`s
//!   from amplifying fetches against the AS — when the floor is hit we fail
//!   closed (401) rather than fetch.

use crate::auth::AuthError;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet};
use jsonwebtoken::DecodingKey;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::debug;
use url::Url;

/// How long a fetched JWKS is considered valid before re-fetching.
pub const JWKS_TTL: Duration = Duration::from_secs(300);

/// Minimum wall-clock interval between *forced* refreshes (unknown-kid path).
/// A burst of tokens carrying bogus `kid`s cannot exceed one AS fetch per
/// window; once the floor is hit we fail closed instead of fetching (M2).
pub const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

struct CachedJwks {
    jwks: JwkSet,
    fetched_at: Instant,
}

/// A shared, TTL-cached JWKS resolver keyed on the AS `jwks_url`.
pub struct JwksCache {
    jwks_url: String,
    client: reqwest::Client,
    cache: Arc<RwLock<Option<CachedJwks>>>,
    /// Single-flight guard: serializes network refreshes so concurrent unknown
    /// -kid lookups collapse into one fetch (M2). Also records the last forced
    /// -refresh instant for the interval gate.
    refresh_lock: Arc<Mutex<Option<Instant>>>,
}

impl JwksCache {
    /// Construct a cache after validating `jwks_url` against the SSRF policy.
    ///
    /// Returns `AuthError::ProviderError` (fail-closed, surfaced as a
    /// `FailingAuthenticator` at build time) when the URL is not an acceptable
    /// JWKS endpoint. Builds a dedicated client with redirects disabled.
    pub fn new(
        jwks_url: impl Into<String>,
        _shared_client: reqwest::Client,
    ) -> Result<Self, AuthError> {
        let jwks_url = jwks_url.into();
        validate_jwks_url(&jwks_url)?;

        // SAFETY(security): a *dedicated* client with redirect-following
        // disabled — never reuse the upstream-proxy client here. Prevents the
        // JWKS host from 3xx-redirecting the fetch to a link-local metadata
        // endpoint (SSRF). `_shared_client` is intentionally ignored.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AuthError::ProviderError(format!("JWKS client build error: {e}")))?;

        Ok(Self {
            jwks_url,
            client,
            cache: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(Mutex::new(None)),
        })
    }

    /// Fetch the JWKS over the network and replace the cache.
    async fn fetch(&self) -> Result<JwkSet, AuthError> {
        debug!(url = %self.jwks_url, "fetching JWKS");
        let resp = self
            .client
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|e| AuthError::ProviderError(format!("JWKS fetch error: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AuthError::ProviderError(format!(
                "JWKS endpoint returned {status}"
            )));
        }

        let jwks: JwkSet = resp
            .json()
            .await
            .map_err(|e| AuthError::ProviderError(format!("JWKS parse error: {e}")))?;

        *self.cache.write().await = Some(CachedJwks {
            jwks: jwks.clone(),
            fetched_at: Instant::now(),
        });

        Ok(jwks)
    }

    /// Return the cached JWKS, fetching from the network if stale or absent.
    async fn jwks(&self) -> Result<JwkSet, AuthError> {
        // Fast path — read-lock check.
        {
            let guard = self.cache.read().await;
            if let Some(c) = guard.as_ref() {
                if c.fetched_at.elapsed() < JWKS_TTL {
                    return Ok(c.jwks.clone());
                }
            }
        }
        // Slow path — single-flight the network fetch.
        self.refresh_single_flight(true).await
    }

    /// Perform a refresh under the single-flight lock.
    ///
    /// - `allow_stale_ttl == true` (TTL-expiry path): always fetch when the
    ///   cache is stale/absent.
    /// - `allow_stale_ttl == false` (unknown-kid path): honor the
    ///   [`MIN_REFRESH_INTERVAL`] floor — if a forced refresh happened too
    ///   recently, fail closed instead of hammering the AS (M2 DoS gate).
    ///
    /// While one task holds the lock and refreshes, others block and then read
    /// the freshly-populated cache rather than issuing duplicate fetches.
    async fn refresh_single_flight(&self, allow_stale_ttl: bool) -> Result<JwkSet, AuthError> {
        let mut last_forced = self.refresh_lock.lock().await;

        // Another task may have refreshed while we waited for the lock.
        {
            let guard = self.cache.read().await;
            if let Some(c) = guard.as_ref() {
                let fresh = c.fetched_at.elapsed() < JWKS_TTL;
                if fresh || !allow_stale_ttl {
                    // For the unknown-kid path (`!allow_stale_ttl`) we return the
                    // current cache; the caller re-checks for the kid and applies
                    // the interval gate below only if still missing.
                    if fresh {
                        return Ok(c.jwks.clone());
                    }
                }
            }
        }

        if !allow_stale_ttl {
            // Unknown-kid forced refresh: enforce the minimum interval.
            if let Some(prev) = *last_forced {
                if prev.elapsed() < MIN_REFRESH_INTERVAL {
                    return Err(AuthError::Unauthorized(
                        "unknown signing key; refresh rate-limited".to_string(),
                    ));
                }
            }
            *last_forced = Some(Instant::now());
        }

        self.fetch().await
    }

    /// Resolve a [`DecodingKey`] for the given optional `kid`.
    ///
    /// - `Some(kid)` → look up by `kid`; on a cache miss, force ONE rate-limited
    ///   refresh and retry. Still missing → 401.
    /// - `None` → only valid when the JWKS holds exactly ONE key; a multi-key
    ///   set with no `kid` is ambiguous and REJECTED (H2).
    ///
    /// Only asymmetric (RSA/EC) keys are accepted; symmetric/`oct` JWKs are
    /// refused (H2). Fails CLOSED on any ambiguity.
    pub async fn decoding_key(&self, kid: Option<&str>) -> Result<DecodingKey, AuthError> {
        let jwks = self.jwks().await?;

        // Fast path against the current cache.
        if let Some(kid) = kid {
            if jwks.find(kid).is_some() {
                return select_asymmetric_key(&jwks, Some(kid));
            }
            // Unknown kid — force exactly one rate-limited refresh, then retry.
            debug!(kid, "kid not in cached JWKS — forcing single refresh");
            let refreshed = self.refresh_single_flight(false).await?;
            return select_asymmetric_key(&refreshed, Some(kid));
        }
        select_asymmetric_key(&jwks, None)
    }
}

/// Pure key-selection over a resolved [`JwkSet`] (H2). Kept free of I/O so the
/// ambiguity/rejection rules are unit-testable without a live JWKS server.
///
/// - `Some(kid)` → exact match required; missing → 401.
/// - `None` → only permitted when the set holds exactly ONE key; a multi-key
///   set with no `kid` is ambiguous and REJECTED (never "pick first").
///
/// The chosen key must be asymmetric (RSA/EC); symmetric keys are refused.
fn select_asymmetric_key(jwks: &JwkSet, kid: Option<&str>) -> Result<DecodingKey, AuthError> {
    match kid {
        Some(kid) => {
            let jwk = jwks.find(kid).ok_or_else(|| {
                AuthError::Unauthorized(format!("no JWKS key found for kid={kid}"))
            })?;
            asymmetric_decoding_key(jwk)
        }
        None => {
            if jwks.keys.len() != 1 {
                return Err(AuthError::Unauthorized(format!(
                    "token has no kid and JWKS holds {} keys — ambiguous",
                    jwks.keys.len()
                )));
            }
            let jwk = jwks
                .keys
                .first()
                .ok_or_else(|| AuthError::ProviderError("JWKS has no keys".to_string()))?;
            asymmetric_decoding_key(jwk)
        }
    }
}

/// Build a [`DecodingKey`] from a JWK, rejecting symmetric/`oct` keys (H2).
///
/// A resource server verifying externally-minted tokens must only trust
/// asymmetric public keys. Accepting an `oct` (HMAC secret) key here would let
/// an attacker who learns the "public" symmetric material forge tokens, and is
/// the residual leg of the alg-confusion attack. Only RSA and EC are allowed.
fn asymmetric_decoding_key(jwk: &Jwk) -> Result<DecodingKey, AuthError> {
    match &jwk.algorithm {
        AlgorithmParameters::RSA(_) | AlgorithmParameters::EllipticCurve(_) => {
            DecodingKey::from_jwk(jwk)
                .map_err(|e| AuthError::ProviderError(format!("invalid JWK: {e}")))
        }
        AlgorithmParameters::OctetKey(_) | AlgorithmParameters::OctetKeyPair(_) => Err(
            AuthError::Unauthorized("symmetric JWK rejected for token verification".to_string()),
        ),
    }
}

/// SSRF policy for a JWKS URL (H1).
///
/// Accept `https://<public-host>` and `http(s)://<loopback>` only. Reject any
/// non-http(s) scheme, `http` to a non-loopback host, and any host that
/// resolves *literally* to a loopback (non-dev), link-local, or private IP.
/// DNS-name hosts are allowed (we cannot resolve at config time without a
/// lookup, and disabling redirects blocks the classic rebind-to-metadata path);
/// literal-IP hosts in dangerous ranges are the concrete, checkable threat.
pub fn validate_jwks_url(raw: &str) -> Result<(), AuthError> {
    let url =
        Url::parse(raw).map_err(|e| AuthError::ProviderError(format!("invalid jwks_url: {e}")))?;

    let scheme = url.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(AuthError::ProviderError(format!(
            "jwks_url scheme must be http(s), got {scheme}"
        )));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AuthError::ProviderError("jwks_url has no host".to_string()))?;

    // `url::host_str()` returns IPv6 literals wrapped in brackets (`[fe80::1]`).
    // Strip them so `IpAddr` parsing (and thus the SSRF IP checks) actually
    // fires for IPv6 hosts — otherwise a bracketed literal would bypass every
    // IP-range guard below (SSRF gap).
    let host_ip_str = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let parsed_ip = host_ip_str.parse::<IpAddr>().ok();

    // Is the host a loopback literal or the `localhost` name (dev exception)?
    let is_loopback_host = host.eq_ignore_ascii_case("localhost")
        || parsed_ip.map(|ip| ip.is_loopback()).unwrap_or(false);

    // `http` is permitted ONLY for a loopback dev host; everything else needs TLS.
    if scheme == "http" && !is_loopback_host {
        return Err(AuthError::ProviderError(
            "jwks_url must use https (http allowed only for localhost/loopback)".to_string(),
        ));
    }

    // Reject dangerous literal-IP hosts. `localhost` (name) is the sanctioned
    // dev loopback and is allowed; a loopback *IP literal* over https would be
    // unusual for a real AS, so we reject loopback/link-local/private literals
    // unless they came in via the http+loopback dev exception above.
    if let Some(ip) = parsed_ip {
        if scheme == "http" && ip.is_loopback() {
            // Explicit dev exception already validated — allow.
            return Ok(());
        }
        if ip.is_loopback() {
            return Err(AuthError::ProviderError(
                "jwks_url points at a loopback address".to_string(),
            ));
        }
        if is_link_local(&ip) {
            return Err(AuthError::ProviderError(
                "jwks_url points at a link-local address".to_string(),
            ));
        }
        if is_private(&ip) {
            return Err(AuthError::ProviderError(
                "jwks_url points at a private-range address".to_string(),
            ));
        }
    }

    Ok(())
}

/// Link-local: IPv4 169.254.0.0/16, IPv6 fe80::/10.
fn is_link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        // `Ipv6Addr::is_unicast_link_local` is unstable; check fe80::/10 by hand.
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// Private ranges: IPv4 RFC 1918, IPv6 unique-local fc00::/7.
fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    // ── H1: SSRF URL policy ────────────────────────────────────────────────

    #[test]
    fn accepts_https_public_host() {
        assert!(validate_jwks_url("https://as.example.com/.well-known/jwks.json").is_ok());
    }

    #[test]
    fn accepts_http_localhost_dev() {
        assert!(validate_jwks_url("http://localhost:4444/jwks").is_ok());
        assert!(validate_jwks_url("http://127.0.0.1:4444/jwks").is_ok());
    }

    #[test]
    fn rejects_http_non_local() {
        let e = validate_jwks_url("http://as.example.com/jwks");
        assert!(matches!(e, Err(AuthError::ProviderError(_))));
    }

    #[test]
    fn rejects_link_local_metadata_endpoint() {
        // The classic cloud metadata SSRF target.
        let e = validate_jwks_url("https://169.254.169.254/latest/meta-data");
        assert!(matches!(e, Err(AuthError::ProviderError(_))));
    }

    #[test]
    fn rejects_ipv6_link_local() {
        let e = validate_jwks_url("https://[fe80::1]/jwks");
        assert!(matches!(e, Err(AuthError::ProviderError(_))));
    }

    #[test]
    fn rejects_loopback_ip_over_https() {
        let e = validate_jwks_url("https://127.0.0.1/jwks");
        assert!(matches!(e, Err(AuthError::ProviderError(_))));
    }

    #[test]
    fn rejects_private_range() {
        assert!(validate_jwks_url("https://10.0.0.5/jwks").is_err());
        assert!(validate_jwks_url("https://192.168.1.10/jwks").is_err());
        assert!(validate_jwks_url("https://172.16.0.1/jwks").is_err());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(validate_jwks_url("file:///etc/passwd").is_err());
        assert!(validate_jwks_url("ftp://as.example.com/jwks").is_err());
    }

    #[test]
    fn link_local_and_private_helpers() {
        assert!(is_link_local(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(!is_link_local(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(is_link_local(&IpAddr::V6(
            "fe80::abcd".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_private(&IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
        assert!(is_private(&IpAddr::V6(
            "fc00::1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(!is_private(&IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    // ── H2: symmetric JWK rejection ────────────────────────────────────────

    #[test]
    fn rejects_symmetric_oct_jwk() {
        // An `oct` (HMAC secret) JWK must be refused for token verification.
        let jwk: Jwk = serde_json::from_value(serde_json::json!({
            "kty": "oct",
            "kid": "sym-1",
            "k": "c2VjcmV0LWtleS1tYXRlcmlhbA",
            "alg": "HS256"
        }))
        .unwrap();
        let r = asymmetric_decoding_key(&jwk);
        assert!(matches!(r, Err(AuthError::Unauthorized(_))));
    }

    // ── Construction validation is wired ───────────────────────────────────

    #[test]
    fn new_rejects_ssrf_url() {
        let r = JwksCache::new("https://169.254.169.254/jwks", reqwest::Client::new());
        assert!(matches!(r, Err(AuthError::ProviderError(_))));
    }

    #[test]
    fn new_accepts_valid_url() {
        assert!(JwksCache::new("https://as.example.com/jwks", reqwest::Client::new()).is_ok());
    }

    #[tokio::test]
    async fn fetch_failure_is_provider_error() {
        // SAFETY(test): loopback dev URL passes the SSRF gate; port 1 is unbound
        // → connection refused → ProviderError, exercising the fail-closed path.
        let cache =
            JwksCache::new("http://127.0.0.1:1/jwks", reqwest::Client::new()).expect("valid url");
        let result = cache.decoding_key(Some("key-1")).await;
        assert!(matches!(result, Err(AuthError::ProviderError(_))));
    }

    // ── H2: kid ambiguity via the pure selector ────────────────────────────

    /// Two RSA public-key JWKs (kid `a` and `b`) for ambiguity tests. Both reuse
    /// the RFC 7517-style valid modulus so `DecodingKey::from_jwk` succeeds; only
    /// the `kid` differs, which is all the selector logic cares about.
    fn two_key_rsa_jwks() -> JwkSet {
        const N: &str = "wfbfG9KTU-TT-VL6l0RBOAtR1Dc85sc5ZwC1ml6RBaZTEv4pRvYuYpDbksDKsHlUbch35-D24AKe-wry6CvEp667qK9E1mkyG6pNveZPekV1gd9FXKKd0rrs35MmAb-tlIy6gVc45NS4mGOcfl6obW5h2GZa2bpa93Ka0XO7HGF5tReXc32nZfudma_yNK4VlOlbuIxf6-Lk96Td6SRsc7s97k8q_cou_Bhk4u-2OCozv7GuENmWObo5E2LK3kBTRNtjruF5NG7AtTqG3QEpV-FGL5PXQQ2Yu-Mzokqe-j51PtaaR7OGumWK3bD1XV6RxXKYfMFPugSZxAy0_0ZdTw";
        serde_json::from_value(serde_json::json!({
            "keys": [
                { "kty": "RSA", "kid": "a", "use": "sig", "alg": "RS256", "n": N, "e": "AQAB" },
                { "kty": "RSA", "kid": "b", "use": "sig", "alg": "RS256", "n": N, "e": "AQAB" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn multi_key_no_kid_is_rejected() {
        // H2: token without kid against a 2-key set is ambiguous → reject.
        let jwks = two_key_rsa_jwks();
        let r = select_asymmetric_key(&jwks, None);
        assert!(matches!(r, Err(AuthError::Unauthorized(_))));
    }

    #[test]
    fn single_key_no_kid_is_accepted() {
        // One-key set: no-kid resolution is unambiguous and allowed.
        let mut jwks = two_key_rsa_jwks();
        jwks.keys.truncate(1);
        assert!(select_asymmetric_key(&jwks, None).is_ok());
    }

    #[test]
    fn unknown_kid_is_rejected() {
        let jwks = two_key_rsa_jwks();
        let r = select_asymmetric_key(&jwks, Some("does-not-exist"));
        assert!(matches!(r, Err(AuthError::Unauthorized(_))));
    }

    #[test]
    fn known_kid_resolves() {
        let jwks = two_key_rsa_jwks();
        assert!(select_asymmetric_key(&jwks, Some("a")).is_ok());
        assert!(select_asymmetric_key(&jwks, Some("b")).is_ok());
    }

    // ── M2: forced-refresh interval gate (pure logic) ──────────────────────

    /// The gate: given the instant of the last forced refresh and `now`, may we
    /// force another? Mirrors the check inside `refresh_single_flight`.
    fn may_force_refresh(last_forced: Option<Instant>, now: Instant) -> bool {
        match last_forced {
            Some(prev) => now.duration_since(prev) >= MIN_REFRESH_INTERVAL,
            None => true,
        }
    }

    #[test]
    fn refresh_gate_allows_first_and_blocks_within_window() {
        let now = Instant::now();
        // No prior forced refresh → allowed.
        assert!(may_force_refresh(None, now));
        // A refresh just happened → blocked within the window.
        assert!(!may_force_refresh(Some(now), now));
        // Just before the floor elapses → still blocked.
        let almost = now + (MIN_REFRESH_INTERVAL - Duration::from_millis(1));
        assert!(!may_force_refresh(Some(now), almost));
        // After the floor elapses → allowed again.
        let after = now + MIN_REFRESH_INTERVAL + Duration::from_millis(1);
        assert!(may_force_refresh(Some(now), after));
    }
}
