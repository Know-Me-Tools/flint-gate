# authorization-policy

## ADDED Requirements

### Requirement: Embedded policy-based authorization
The gateway SHALL evaluate route-level authorization decisions through an embedded Cedar policy engine, with policies loaded from the database at runtime and evaluated on the request hot path.

#### Scenario: Policy allows a request
- **WHEN** an `Authorize` pre-request hook evaluates and the active Cedar policy set returns `Allow` for the request (principal, action, resource, context)
- **THEN** the request proceeds to the upstream

#### Scenario: Policy denies a request
- **WHEN** the active policy set returns `Deny` (or no explicit `Allow`) and the hook's `enforce` is true
- **THEN** the request is blocked with HTTP 403 `{"error":"forbidden","message":...}`

#### Scenario: Evaluation fails closed
- **WHEN** authorization cannot be evaluated (malformed principal, non-object context, schema-mismatched request, or any engine error)
- **THEN** the decision is `Deny` — there is no path to `Allow` other than an explicit Cedar `Allow`

#### Scenario: Empty policy set denies by default
- **WHEN** no enabled policies are configured
- **THEN** the engine default-denies every authorization request

#### Scenario: Shadow mode does not weaken enforcement
- **WHEN** an `Authorize` hook is configured with `enforce=false`
- **THEN** a would-be denial is logged only and the request proceeds, without affecting enforcement on any other route (default `enforce` is true)

### Requirement: Safe policy hot-reload
The gateway SHALL reload policies without restart using parse-before-swap semantics, retaining the last-good policy set on failure and isolating individual bad rows.

#### Scenario: Good reload swaps atomically
- **WHEN** the policy store changes and a reload compiles successfully
- **THEN** the live policy bundle is atomically replaced via a lock-free swap with no torn reads on the hot path

#### Scenario: Bad reload retains last-good
- **WHEN** a reload fails to compile the new bundle
- **THEN** the previously active bundle is retained and the error is logged; the engine never swaps to an empty or permissive set on error

#### Scenario: One poisoned row does not disable all authorization
- **WHEN** the policy store contains one malformed row alongside valid rows (at startup or reload)
- **THEN** the malformed row is skipped and logged, and the engine is built from the surviving valid rows

#### Scenario: Multi-replica reload propagates
- **WHEN** a policy is written on one replica
- **THEN** a database notification triggers every replica to reload from the database, so peers do not serve stale (over-permissive) decisions

### Requirement: Write-time policy validation and administration
The gateway SHALL validate policies before persisting them and expose policy administration only on the private admin surface.

#### Scenario: Invalid policy is rejected before storage
- **WHEN** an admin create/update request submits policy text, schema, or entities that fail Cedar validation
- **THEN** the request is rejected with HTTP 400 and the validation error, and nothing is persisted

#### Scenario: Non-activating write is surfaced, not silent
- **WHEN** a validated policy is stored but the subsequent reload fails to activate it
- **THEN** the admin response is a non-2xx error indicating the policy was stored but not activated

#### Scenario: Broad allow-all policy is flagged
- **WHEN** a submitted `permit` is unconditional and unconstrained (allow-all)
- **THEN** the response includes a non-blocking warning that the policy grants broad access

#### Scenario: Admin surface is private by default
- **WHEN** the gateway starts with default configuration
- **THEN** the admin server binds to loopback (`127.0.0.1`), and policy CRUD is not mounted on the public proxy router
