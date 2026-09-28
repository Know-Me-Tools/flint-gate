# Channel effect authority Specification

## Requirements

### Requirement: Gate-owned scoped grant authority

Gate SHALL persist issuer-scoped channel grants and their revisions independently
of caller-supplied C02 `GrantRef` facts. A grant SHALL qualify exactly one of
source disclosure, recipient delivery, selected handler execution, scoped
reply, or route reassignment for one source scope, recipient, handler, classification, and causal
limit.

#### Scenario: route affinity is reassigned

- **WHEN** BossFang proposes changing a retained route from its current
  revision to a chosen handler
- **THEN** Gate requires an explicit `route_reassignment` grant and Cedar
  permit bound to that expected route revision and chosen recipient/handler;
  BossFang may perform its own route-state compare-and-swap only after a
  fresh release receipt

#### Scenario: a grant is revoked before queue release

- **WHEN** an effect was evaluated as eligible and the matching grant is revoked
  before `/authority/channels/release`
- **THEN** release returns `withheld` with the current grant revision and does
  not return a release permit

#### Scenario: a caller asserts a stale grant revision

- **WHEN** a caller attempts to use old or self-asserted grant state
- **THEN** Gate ignores that assertion as authority and resolves the current
  Postgres row and Cedar policy itself

### Requirement: exact channel-effect receipts

Gate SHALL bind each decision to the stable source occurrence, action, provider
scope, recipient, handler, route revision, payload digest, original principal,
authenticated execution owner, and causal budget. Evaluation and release SHALL
have different receipt IDs and report contract, grant revision, policy set,
policy revision, and digest.

#### Scenario: a queued effect is released

- **WHEN** the exact prior evaluation was eligible and the current grant and
  Cedar authority still match
- **THEN** Gate records a separate `released` receipt for that exact effect

#### Scenario: release is replayed

- **WHEN** a release receipt already exists for the effect
- **THEN** Gate reports `uncertain` and does not issue a second release permit

#### Scenario: authority backend is absent

- **WHEN** Postgres is not configured for Gate
- **THEN** capabilities report unavailable and channel grant/effect requests
  fail closed without changing the existing C02 API
