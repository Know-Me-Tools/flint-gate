//! Authoritative, shared fixed-window counters backed by Redis.
//!
//! This module provides cross-replica enforcement for two concerns:
//!
//! - **Token budgets** — keyed `budget:{scope}:{id}:{window}`, incremented by
//!   the token count consumed by a request.
//! - **Request-rate limits** — keyed `ratelimit:{scope}:{id}:{window}`,
//!   incremented by one per request.
//!
//! Both share a single atomic fixed-window primitive: a Lua script that runs
//! `INCR` then sets `EXPIRE` (only on the first increment, i.e. when the key was
//! just created) so the window naturally rolls over. Running `INCR` + `EXPIRE`
//! inside one Lua script guarantees atomicity and avoids a leaked key that never
//! expires (the classic `INCR` without-`EXPIRE` race).
//!
//! The whole module is gated behind the `redis-l2` feature. When the feature is
//! off there is no shared counter available and callers fall back to the
//! Postgres time-bounded sum (see `Database::get_user_token_total_windowed`).

pub mod governor_layer;

pub use governor_layer::{build_governor_layer, CredentialKeyExtractor, RateLimitKey};

/// Errors raised by the Redis-backed window counters.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    /// The underlying Redis command failed.
    #[cfg(feature = "redis-l2")]
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    /// The window was `Lifetime`, which has no fixed TTL and is not supported
    /// by the fixed-window counters. Callers must handle `Lifetime` separately.
    #[error("lifetime window has no fixed-window counter")]
    UnsupportedWindow,
}

#[cfg(feature = "redis-l2")]
mod redis_impl {
    use super::RateLimitError;
    use crate::config::types::{BudgetScope, BudgetWindow};
    use redis::aio::ConnectionManager;
    use redis::AsyncCommands;
    use std::sync::OnceLock;

    /// Atomic fixed-window increment.
    ///
    /// `KEYS[1]` = counter key, `ARGV[1]` = increment amount, `ARGV[2]` = TTL
    /// seconds. Sets the TTL only when the counter was just created (return of
    /// `INCRBY` equals the increment amount), so the window is anchored to the
    /// first hit and rolls over cleanly. Returns the current counter value.
    const WINDOW_INCR_LUA: &str = r#"
local current = redis.call('INCRBY', KEYS[1], ARGV[1])
if current == tonumber(ARGV[1]) then
    redis.call('EXPIRE', KEYS[1], ARGV[2])
end
return current
"#;

    fn window_script() -> &'static redis::Script {
        static SCRIPT: OnceLock<redis::Script> = OnceLock::new();
        SCRIPT.get_or_init(|| redis::Script::new(WINDOW_INCR_LUA))
    }

    /// Build the token-budget counter key: `flint:budget:{scope}:{id}:{window}`.
    pub(super) fn budget_key(scope: BudgetScope, id: &str, window: BudgetWindow) -> String {
        format!("flint:budget:{}:{}:{}", scope.tag(), id, window.tag())
    }

    /// Build the request-rate counter key: `flint:ratelimit:{scope}:{id}:{window}`.
    pub(super) fn ratelimit_key(scope: BudgetScope, id: &str, window: BudgetWindow) -> String {
        format!("flint:ratelimit:{}:{}:{}", scope.tag(), id, window.tag())
    }

    /// Resolve the fixed-window TTL, mapping `Lifetime` to the typed error.
    pub(super) fn window_ttl(window: BudgetWindow) -> Result<u64, RateLimitError> {
        window
            .duration_secs()
            .ok_or(RateLimitError::UnsupportedWindow)
    }

    /// Redis-backed fixed-window counter handle. Cheap to clone (wraps a
    /// `ConnectionManager`, which is itself a cheap clone of a shared pool).
    #[derive(Clone)]
    pub struct RedisRateLimiter {
        conn: ConnectionManager,
    }

    impl RedisRateLimiter {
        /// Wrap an existing connection manager (shared with the L2 cache).
        pub fn new(conn: ConnectionManager) -> Self {
            Self { conn }
        }

        /// Increment a token-budget counter by `tokens` and return the new
        /// window total. Key: `flint:budget:{scope}:{id}:{window}`.
        pub async fn incr_budget(
            &self,
            scope: BudgetScope,
            id: &str,
            window: BudgetWindow,
            tokens: u64,
        ) -> Result<u64, RateLimitError> {
            let ttl = window_ttl(window)?;
            let key = budget_key(scope, id, window);
            self.incr(&key, tokens, ttl).await
        }

        /// Read the current token-budget window total without incrementing.
        /// Returns `0` when the counter key has expired or never existed. Used
        /// by the pre-request budget check (the counter is advanced later, at
        /// metering time, by [`incr_budget`](Self::incr_budget)).
        pub async fn get_budget(
            &self,
            scope: BudgetScope,
            id: &str,
            window: BudgetWindow,
        ) -> Result<u64, RateLimitError> {
            window_ttl(window)?; // validate window; Lifetime → UnsupportedWindow
            let key = budget_key(scope, id, window);
            let mut conn = self.conn.clone();
            let value: Option<i64> = conn.get(&key).await?;
            Ok(value.unwrap_or(0).max(0) as u64)
        }

        /// Increment a request-rate counter by one and return the new window
        /// total. Key: `flint:ratelimit:{scope}:{id}:{window}`.
        #[allow(dead_code)]
        pub async fn incr_request(
            &self,
            scope: BudgetScope,
            id: &str,
            window: BudgetWindow,
        ) -> Result<u64, RateLimitError> {
            let ttl = window_ttl(window)?;
            let key = ratelimit_key(scope, id, window);
            self.incr(&key, 1, ttl).await
        }

        /// Run the atomic INCRBY+EXPIRE script and return the current value.
        async fn incr(&self, key: &str, amount: u64, ttl: u64) -> Result<u64, RateLimitError> {
            let mut conn = self.conn.clone();
            let current: i64 = window_script()
                .key(key)
                .arg(amount)
                .arg(ttl)
                .invoke_async(&mut conn)
                .await?;
            Ok(current.max(0) as u64)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::config::types::{BudgetScope, BudgetWindow};

        #[test]
        fn budget_key_has_stable_scope_id_window_shape() {
            assert_eq!(
                budget_key(BudgetScope::User, "user-42", BudgetWindow::Hour),
                "flint:budget:user:user-42:hour"
            );
            assert_eq!(
                budget_key(BudgetScope::Team, "team-7", BudgetWindow::Day),
                "flint:budget:team:team-7:day"
            );
        }

        #[test]
        fn ratelimit_key_has_stable_scope_id_window_shape() {
            assert_eq!(
                ratelimit_key(BudgetScope::User, "u1", BudgetWindow::Minute),
                "flint:ratelimit:user:u1:minute"
            );
        }

        #[test]
        fn window_ttl_maps_fixed_windows_to_seconds() {
            assert_eq!(window_ttl(BudgetWindow::Minute).unwrap(), 60);
            assert_eq!(window_ttl(BudgetWindow::Hour).unwrap(), 3_600);
            assert_eq!(window_ttl(BudgetWindow::Day).unwrap(), 86_400);
        }

        #[test]
        fn window_ttl_rejects_lifetime_with_unsupported_window_error() {
            let err = window_ttl(BudgetWindow::Lifetime).unwrap_err();
            assert!(matches!(err, RateLimitError::UnsupportedWindow));
            assert_eq!(
                err.to_string(),
                "lifetime window has no fixed-window counter"
            );
        }

        #[test]
        fn window_script_is_incrby_then_conditional_expire() {
            // The Lua text is the atomic primitive — guard its INCRBY+EXPIRE shape
            // so a regression that drops the EXPIRE (leaking un-expiring keys) or
            // the increment is caught without a live server.
            assert!(WINDOW_INCR_LUA.contains("INCRBY"));
            assert!(WINDOW_INCR_LUA.contains("EXPIRE"));
            // EXPIRE is guarded so it only fires on first creation of the window.
            assert!(WINDOW_INCR_LUA.contains("if current == tonumber(ARGV[1]) then"));
        }

        // ── Live-Redis integration tests ──────────────────────────────────
        // These require a real Redis at redis://127.0.0.1:6379 and are #[ignore]d
        // so the default `cargo test` run needs no server. Run explicitly with:
        //   cargo test -p flint-gate-core --all-features -- --ignored
        async fn test_limiter() -> RedisRateLimiter {
            let client = redis::Client::open("redis://127.0.0.1:6379").unwrap();
            let conn = client.get_connection_manager().await.unwrap();
            RedisRateLimiter::new(conn)
        }

        #[tokio::test]
        #[ignore = "requires a live Redis server"]
        async fn incr_budget_accumulates_within_window() {
            let limiter = test_limiter().await;
            let id = format!("test-{}", uuid::Uuid::new_v4());
            let first = limiter
                .incr_budget(BudgetScope::User, &id, BudgetWindow::Minute, 100)
                .await
                .unwrap();
            let second = limiter
                .incr_budget(BudgetScope::User, &id, BudgetWindow::Minute, 50)
                .await
                .unwrap();
            assert_eq!(first, 100);
            assert_eq!(second, 150);
            assert_eq!(
                limiter
                    .get_budget(BudgetScope::User, &id, BudgetWindow::Minute)
                    .await
                    .unwrap(),
                150
            );
        }

        #[tokio::test]
        #[ignore = "requires a live Redis server"]
        async fn get_budget_returns_zero_for_unknown_key() {
            let limiter = test_limiter().await;
            let id = format!("absent-{}", uuid::Uuid::new_v4());
            assert_eq!(
                limiter
                    .get_budget(BudgetScope::User, &id, BudgetWindow::Hour)
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[cfg(feature = "redis-l2")]
pub use redis_impl::RedisRateLimiter;
