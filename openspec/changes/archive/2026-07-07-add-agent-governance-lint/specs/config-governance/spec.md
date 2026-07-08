# config-governance

## ADDED Requirements

### Requirement: Startup agent-governance lint
The gateway SHALL, at startup, lint the loaded configuration for
**under-governed agent-reachable routes** and surface each finding, so an
operator learns about a silent under-application of agent controls before it
takes effect rather than after agent spend has been misaccounted.

A route is **agent-reachable** when its effective auth provider — resolved
exactly as the request pipeline resolves it (`route.auth`, else the route's
site `default_auth`, mapped through `auth_providers` to a provider variant) —
is JWKS-backed (`jwt` or `mcp`), because only such a provider can carry an
RFC 8693 `act` / agent-classified token.

#### Scenario: Agent-reachable route with a non-agent-scoped budget
- **WHEN** an agent-reachable route carries a token-budget hook whose `scope`
  is not `agent`
- **THEN** the lint emits a finding for that route naming the non-agent-scoped
  budget (agent spend would be accounted in a non-agent keyspace)

#### Scenario: Agent-reachable route with no authorization hook
- **WHEN** an agent-reachable route has no per-tool `authorize` hook
- **THEN** the lint emits a finding for that route (its tool calls are
  ungoverned)

#### Scenario: Lifetime agent budget is always flagged
- **WHEN** any route carries a token-budget hook with `scope: agent` and
  `window: lifetime`
- **THEN** the lint emits a finding regardless of the route's reachability,
  because a lifetime window cannot fail closed on a counter-store outage

#### Scenario: Route naming an undefined auth provider
- **WHEN** a route (or its site default) names an auth provider that is not
  defined in `auth_providers`
- **THEN** the lint emits an unresolvable-provider finding, because that route
  fails at request time and its agent-reachability cannot be established

#### Scenario: A fully governed agent route is clean
- **WHEN** an agent-reachable route has an agent-scoped, non-lifetime budget
  and an authorize hook
- **THEN** the lint emits no finding for that route

#### Scenario: Findings are de-duplicated
- **WHEN** a single route would produce the same finding reason more than once
- **THEN** the lint reports that `(route, reason)` pair only once

### Requirement: Governance lint severity is operator-controlled
The gateway SHALL treat lint findings as **advisory by default** and provide an
opt-in strict mode that refuses to start when any finding exists, so that
upgrading never breaks an existing deployment while operators who want a hard
guarantee can demand one.

#### Scenario: Advisory by default
- **WHEN** `server.strict_agent_governance` is unset or false and the lint
  produces findings
- **THEN** each finding is logged as a warning and startup proceeds

#### Scenario: Strict mode refuses to start
- **WHEN** `server.strict_agent_governance` is true and the lint produces at
  least one finding
- **THEN** startup fails with an error enumerating the findings

#### Scenario: Strict mode with a clean config starts normally
- **WHEN** `server.strict_agent_governance` is true and the lint produces no
  findings
- **THEN** startup proceeds normally
