//! Shared, cross-replica rate limiting for the `/oauth/*` endpoints.
//!
//! Unlike the in-process `governor` layer (per-replica, `quanta`-clock), this
//! consults the authoritative Redis fixed-window counter
//! ([`RedisRateLimiter::incr_request`]) so a horizontally-scaled deployment
//! enforces one shared limit across all replicas — the exposure gate for
//! scaling `/oauth/token` + `/oauth/introspect` out.
//!
//! The caller key prefers the **authenticated client identity** (`client_id`,
//! from the form body or HTTP Basic) so the limit binds the client-credentials
//! guessing surface and cannot be evaded by rotating the raw Authorization
//! header. It falls back to a hash of the credential header, and finally a
//! shared anonymous bucket for fully unauthenticated callers (client-IP is not
//! available at the handler layer, so the in-process governor provides the
//! per-replica IP-keyed shield for that case). The handlers call
//! [`OAuthLimiter::check`] before doing any work.
//!
//! The whole module is gated behind `redis-l2`.
#![cfg(feature = "redis-l2")]

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::config::types::{BackendUnavailablePosture, BudgetScope, BudgetWindow};
use crate::ratelimit::RedisRateLimiter;

/// Fixed window used for the OAuth request-rate counter. Minute is the coarsest
/// window that still bounds a burst; the effective ceiling is `per_second * 60`.
const OAUTH_WINDOW: BudgetWindow = BudgetWindow::Minute;
const WINDOW_SECS: u64 = 60;

/// Anonymous bucket id when no credential header is present.
const ANON_KEY: &str = "anon";

/// Outcome of a shared-limiter check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitOutcome {
    /// Within the window — allow.
    Allow,
    /// Over the window — deny with `429`.
    Deny,
    /// The shared backend could not be consulted (Redis error). The caller
    /// applies the posture (deny → `503`; degrade → allow + WARN).
    BackendUnavailable,
}

/// Per-endpoint shared limiter handle.
#[derive(Clone)]
pub struct OAuthLimiter {
    limiter: RedisRateLimiter,
    /// Effective per-minute ceiling (`per_second * 60`, min 1).
    per_window: u64,
    /// Posture when Redis is unreachable mid-request.
    on_unavailable: BackendUnavailablePosture,
    /// Stable label distinguishing the token vs introspect counters so the two
    /// endpoints do not share a window.
    endpoint: &'static str,
}

impl OAuthLimiter {
    /// Build a limiter for one endpoint. `per_second` is the configured rate;
    /// the shared window ceiling is `per_second * 60` (min 1).
    pub fn new(
        limiter: RedisRateLimiter,
        per_second: u64,
        on_unavailable: BackendUnavailablePosture,
        endpoint: &'static str,
    ) -> Self {
        Self {
            limiter,
            per_window: per_second.saturating_mul(WINDOW_SECS).max(1),
            on_unavailable,
            endpoint,
        }
    }

    /// Posture for this endpoint (used by the caller on `BackendUnavailable`).
    pub fn posture(&self) -> BackendUnavailablePosture {
        self.on_unavailable
    }

    /// Derive the caller key. When the endpoint has authenticated a stable
    /// **client identity** (`client_id`, from the form body or HTTP Basic), key
    /// on that — this binds the client-credentials guessing surface so an
    /// attacker cannot get a fresh window by rotating the raw Authorization
    /// header while brute-forcing `client_secret` in the body. Otherwise fall
    /// back to a hash of the credential header (Authorization → API key →
    /// cookie), and finally the shared anonymous bucket.
    fn caller_key(headers: &HeaderMap, client_id: Option<&str>) -> String {
        // Prefer the authenticated client identity (namespaced so a client_id
        // cannot collide with a header hash).
        if let Some(cid) = client_id.map(str::trim).filter(|s| !s.is_empty()) {
            let mut h = Sha256::new();
            h.update(cid.as_bytes());
            return format!("cid:{}", hex::encode(h.finalize()));
        }
        let cred = headers
            .get(axum::http::header::AUTHORIZATION)
            .or_else(|| headers.get("x-api-key"))
            .or_else(|| headers.get(axum::http::header::COOKIE))
            .and_then(|v| v.to_str().ok());
        match cred {
            Some(c) => {
                let mut h = Sha256::new();
                h.update(c.as_bytes());
                format!("hdr:{}", hex::encode(h.finalize()))
            }
            None => ANON_KEY.to_string(),
        }
    }

    /// Check-and-increment the shared counter for this caller. A single atomic
    /// INCR advances the window; over the ceiling → `Deny`. A Redis error
    /// surfaces as `BackendUnavailable` (never a silent allow). `client_id` is
    /// the authenticated client identity when known (binds the guessing surface).
    pub async fn check(&self, headers: &HeaderMap, client_id: Option<&str>) -> LimitOutcome {
        let id = format!("{}:{}", self.endpoint, Self::caller_key(headers, client_id));
        match self
            .limiter
            .incr_request(BudgetScope::User, &id, OAUTH_WINDOW)
            .await
        {
            Ok(count) if count > self.per_window => LimitOutcome::Deny,
            Ok(_) => LimitOutcome::Allow,
            Err(e) => {
                warn!(endpoint = self.endpoint, error = %e,
                    "shared OAuth rate-limit backend unavailable");
                LimitOutcome::BackendUnavailable
            }
        }
    }

    /// Run the shared check and map it to an early `Response` when the request
    /// must be rejected. Returns `None` to proceed (allow, or degrade-on-outage).
    /// `Some(resp)` is a `429` (over-window) or `503` (outage under `Deny`).
    /// `client_id` is the authenticated client identity when known.
    pub async fn reject_response(
        &self,
        headers: &HeaderMap,
        client_id: Option<&str>,
    ) -> Option<Response> {
        self.map_outcome(self.check(headers, client_id).await)
    }

    /// Mapping from a [`LimitOutcome`] to an optional early response, using this
    /// endpoint's posture. Delegates to the free [`posture_response`] so the
    /// fail-closed decision is testable without a live Redis. `None` = proceed.
    fn map_outcome(&self, outcome: LimitOutcome) -> Option<Response> {
        posture_response(outcome, self.on_unavailable)
    }
}

/// Pure mapping: `LimitOutcome` + outage posture → optional early response.
/// `None` means proceed (allow, or degrade-on-outage). `Some` is a `429`
/// (over-window) or a `503` (outage under the fail-closed `Deny` posture).
fn posture_response(
    outcome: LimitOutcome,
    on_unavailable: BackendUnavailablePosture,
) -> Option<Response> {
    match outcome {
        LimitOutcome::Allow => None,
        LimitOutcome::Deny => {
            Some((StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response())
        }
        LimitOutcome::BackendUnavailable => match on_unavailable {
            // Fail-closed: cannot consult the shared limit → refuse.
            BackendUnavailablePosture::Deny => Some(
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "rate-limit backend unavailable",
                )
                    .into_response(),
            ),
            // Degrade: proceed (the in-process governor still applies).
            BackendUnavailablePosture::Degrade => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::BackendUnavailablePosture;

    // Pure decision surface. The Redis-backed check paths are exercised by the
    // #[ignore]d live-Redis integration tests and the change's E2E stack.

    #[test]
    fn per_window_ceiling_is_per_second_times_sixty_min_one() {
        assert_eq!(5u64.saturating_mul(WINDOW_SECS).max(1), 300);
        assert_eq!(0u64.saturating_mul(WINDOW_SECS).max(1), 1);
        assert_eq!(u64::MAX.saturating_mul(WINDOW_SECS).max(1), u64::MAX);
    }

    #[test]
    fn deny_posture_is_the_fail_closed_default() {
        assert_eq!(
            BackendUnavailablePosture::default(),
            BackendUnavailablePosture::Deny
        );
    }

    #[test]
    fn caller_key_prefers_client_id_then_header_then_anon() {
        let mut h = HeaderMap::new();
        // No credential, no client_id → anon bucket.
        assert_eq!(OAuthLimiter::caller_key(&h, None), ANON_KEY);
        // Header present → hashed header key (hdr: prefix).
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Basic abc".parse().unwrap(),
        );
        let hdr_key = OAuthLimiter::caller_key(&h, None);
        assert!(hdr_key.starts_with("hdr:"));
        assert_eq!(OAuthLimiter::caller_key(&h, None), hdr_key); // stable
    }

    #[test]
    fn caller_key_binds_to_client_id_regardless_of_header() {
        // The security fix: same client_id → same key even if the raw
        // Authorization header is rotated (defeats header-rotation bypass).
        let mut h1 = HeaderMap::new();
        h1.insert(axum::http::header::AUTHORIZATION, "Basic aaa".parse().unwrap());
        let mut h2 = HeaderMap::new();
        h2.insert(axum::http::header::AUTHORIZATION, "Basic zzz".parse().unwrap());
        let k1 = OAuthLimiter::caller_key(&h1, Some("client-42"));
        let k2 = OAuthLimiter::caller_key(&h2, Some("client-42"));
        assert_eq!(k1, k2, "same client_id must key identically despite header churn");
        assert!(k1.starts_with("cid:"));
        // Whitespace-only / empty client_id is ignored (falls back to header).
        let k_empty = OAuthLimiter::caller_key(&h1, Some("   "));
        assert!(k_empty.starts_with("hdr:"));
    }

    // ── Fail-closed outcome → response mapping (no Redis needed) ──────────────

    fn status(resp: Option<Response>) -> Option<StatusCode> {
        resp.map(|r| r.status())
    }

    #[test]
    fn allow_proceeds_under_both_postures() {
        assert_eq!(
            status(posture_response(
                LimitOutcome::Allow,
                BackendUnavailablePosture::Deny
            )),
            None
        );
        assert_eq!(
            status(posture_response(
                LimitOutcome::Allow,
                BackendUnavailablePosture::Degrade
            )),
            None
        );
    }

    #[test]
    fn over_window_is_429_regardless_of_posture() {
        assert_eq!(
            status(posture_response(
                LimitOutcome::Deny,
                BackendUnavailablePosture::Deny
            )),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(
            status(posture_response(
                LimitOutcome::Deny,
                BackendUnavailablePosture::Degrade
            )),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
    }

    #[test]
    fn backend_unavailable_denies_under_deny_posture() {
        // Fail-closed: the introspection oracle path (always Deny) must 503 when
        // the shared limiter is unreachable — never silently allow.
        assert_eq!(
            status(posture_response(
                LimitOutcome::BackendUnavailable,
                BackendUnavailablePosture::Deny
            )),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    #[test]
    fn backend_unavailable_degrades_under_degrade_posture() {
        // Token endpoint availability-first: proceed (in-process governor still
        // applies) rather than 503 on a Redis blip.
        assert_eq!(
            status(posture_response(
                LimitOutcome::BackendUnavailable,
                BackendUnavailablePosture::Degrade
            )),
            None
        );
    }

    // ── Live-Redis over-window integration (ignored; needs a server) ──────────
    // Run with: cargo test -p flint-gate-core --all-features -- --ignored

    #[cfg(feature = "redis-l2")]
    async fn live_limiter(per_second: u64, posture: BackendUnavailablePosture) -> OAuthLimiter {
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let conn = client.get_connection_manager().await.unwrap();
        OAuthLimiter::new(RedisRateLimiter::new(conn), per_second, posture, "test")
    }

    #[tokio::test]
    #[ignore = "requires a live Redis server"]
    async fn over_window_denies_after_ceiling() {
        // ceiling = per_second(1) * 60 = 60. A fresh unique credential should
        // allow up to the ceiling then deny.
        let lim = live_limiter(1, BackendUnavailablePosture::Deny).await;
        let mut h = HeaderMap::new();
        let cred = format!("Bearer {}", uuid::Uuid::new_v4());
        h.insert(axum::http::header::AUTHORIZATION, cred.parse().unwrap());
        // First hit allows.
        assert_eq!(lim.check(&h, None).await, LimitOutcome::Allow);
        // Exhaust the window; the (ceiling+1)-th hit must Deny.
        for _ in 0..60 {
            let _ = lim.check(&h, None).await;
        }
        assert_eq!(lim.check(&h, None).await, LimitOutcome::Deny);
    }

    #[tokio::test]
    #[ignore = "requires a live Redis server"]
    async fn distinct_endpoints_do_not_share_a_window() {
        // The same credential on token vs introspect uses separate counters.
        let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
        let conn = client.get_connection_manager().await.unwrap();
        let token = OAuthLimiter::new(
            RedisRateLimiter::new(conn.clone()),
            1,
            BackendUnavailablePosture::Deny,
            "token",
        );
        let introspect =
            OAuthLimiter::new(RedisRateLimiter::new(conn), 1, BackendUnavailablePosture::Deny, "introspect");
        let mut h = HeaderMap::new();
        let cred = format!("Bearer {}", uuid::Uuid::new_v4());
        h.insert(axum::http::header::AUTHORIZATION, cred.parse().unwrap());
        // Exhaust token's window.
        for _ in 0..=60 {
            let _ = token.check(&h, None).await;
        }
        assert_eq!(token.check(&h, None).await, LimitOutcome::Deny);
        // Introspect's window for the same credential is still fresh.
        assert_eq!(introspect.check(&h, None).await, LimitOutcome::Allow);
    }
}
