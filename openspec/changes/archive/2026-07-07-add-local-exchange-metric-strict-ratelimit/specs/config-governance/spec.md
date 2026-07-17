# config-governance

## ADDED Requirements

### Requirement: Strict cross-replica rate-limit mode
The gateway SHALL provide an opt-in strict mode that refuses to start when the
OAuth surface is exposed on a non-loopback bind but no **shared, cross-replica**
rate-limit backend is actually available — so an operator who requires
cross-replica-accurate limits cannot silently degrade to per-replica governance.

A shared backend is **available** only when an L2 cache is enabled with a Redis
URL **and** the build provides the live shared limiter (`redis-l2` feature); a
config that names Redis in a binary compiled without that feature is not a
satisfied requirement.

#### Scenario: Strict mode refuses without a shared backend
- **WHEN** `oauth.rate_limit.require_shared_backend` is true, the OAuth surface is
  exposed on a non-loopback bind with the base guards satisfied, and no shared
  backend is available
- **THEN** the gateway refuses to start

#### Scenario: Strict mode starts when a shared backend is available
- **WHEN** `oauth.rate_limit.require_shared_backend` is true and a shared backend
  is available (L2 enabled + Redis URL, with the `redis-l2` build feature)
- **THEN** the gateway starts

#### Scenario: Strict mode is inert on a loopback bind
- **WHEN** the OAuth surface is bound to loopback
- **THEN** the strict requirement is not enforced (local development starts),
  because the endpoints are not internet-reachable

#### Scenario: Default is non-strict
- **WHEN** `oauth.rate_limit.require_shared_backend` is unset
- **THEN** it defaults to false and startup behavior is unchanged (the
  per-replica governor is accepted)

#### Scenario: The requirement can only tighten the posture
- **WHEN** `require_shared_backend` is evaluated
- **THEN** it can only turn an otherwise-`Enforce` posture into a refusal — it
  never relaxes an existing refusal into a start
