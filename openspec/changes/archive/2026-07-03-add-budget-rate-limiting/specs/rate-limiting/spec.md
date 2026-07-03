# rate-limiting

## ADDED Requirements

### Requirement: Windowed token budgets
The gateway SHALL enforce token budgets over a configurable rolling window (`minute`, `hour`, or `day`) in addition to the existing lifetime budget, scoped per user or per team, and SHALL block requests that exceed the configured limit.

#### Scenario: Windowed budget under limit passes
- **WHEN** a request's accumulated token usage for the configured window and scope is below the `limit`
- **THEN** the request proceeds to the upstream normally

#### Scenario: Windowed budget at or over limit is blocked
- **WHEN** a request's accumulated token usage for the configured window and scope is greater than or equal to the `limit`
- **THEN** the gateway responds with HTTP 429 and a JSON body `{"error":"quota_exceeded","message":<msg>}`

#### Scenario: Lifetime budget behavior is unchanged
- **WHEN** a `max_token_budget` hook is configured with `window` omitted or set to `lifetime`
- **THEN** the gateway enforces the lifetime total exactly as before, using the pre-resolved `usage_budget` lookup

#### Scenario: Existing configuration remains valid
- **WHEN** an existing `max_token_budget` config specifies only `limit` and `user_id_expr`
- **THEN** it deserializes with `window = lifetime` and `scope = user` and behaves identically to prior releases

### Requirement: Request-rate limiting
The gateway SHALL provide an in-process per-replica request-rate limiter on the proxy router, keyed by caller credential with a client-IP fallback, configurable via `server.rate_limit`.

#### Scenario: Requests within rate proceed
- **WHEN** a caller's request rate is within the configured `per_second` and `burst`
- **THEN** requests are forwarded normally

#### Scenario: Requests over rate are throttled
- **WHEN** a caller exceeds the configured `per_second`/`burst`
- **THEN** the gateway rejects the excess requests with HTTP 429

#### Scenario: Rate limiting is opt-in
- **WHEN** `server.rate_limit.enabled` is false or absent
- **THEN** no request-rate limiting layer is applied

### Requirement: Authoritative shared enforcement with graceful fallback
The gateway SHALL use Redis-backed atomic window counters for shared, cross-replica budget and rate enforcement when the `redis-l2` feature is enabled, and SHALL fall back to a Postgres windowed `usage_events` sum when Redis is unavailable, without breaking request handling.

#### Scenario: Redis counters used when available
- **WHEN** the `redis-l2` feature is enabled and Redis is reachable
- **THEN** windowed usage is read from and advanced in the shared Redis counter keyed `flint:budget:{scope}:{id}:{window}`

#### Scenario: Postgres fallback when Redis disabled
- **WHEN** the `redis-l2` feature is disabled
- **THEN** windowed usage is computed from `SELECT COALESCE(SUM(tokens),0) FROM usage_events WHERE user_id = $1 AND created_at > now() - $2::interval`

#### Scenario: Backend errors fail open
- **WHEN** a transient Redis or Postgres error occurs while reading windowed usage
- **THEN** the gateway logs a warning and treats usage as zero rather than hard-blocking live traffic
