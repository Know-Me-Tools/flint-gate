# governed-effect-authority Specification

## Purpose
TBD - created by archiving change afc-c02-governed-effect-authority-v1. Update Purpose after archive.

## Requirements

### Requirement: exact effect authority

Flint Gate SHALL evaluate `afc.governed-effect/1` requests and bind every
decision to the exact canonical request, Gate's active content-addressed Cedar
policy set, and current authority references.

#### Scenario: complete request is permitted

- **WHEN** all required identity and authority facts are present and active Cedar policy permits the request
- **THEN** Gate returns `permit` with the request binding and bound authority references

#### Scenario: authority input is missing or invalid

- **WHEN** protocol, identity, digest, grant, lease, budget, or epoch input is invalid
- **THEN** Gate returns a visible denial and does not return a permit

#### Scenario: policy authority is Gate-resolved

- **WHEN** Gate evaluates or revalidates an effect
- **THEN** it resolves the active policy set, revision, and digest internally rather than trusting a client-provided policy reference

#### Scenario: caller attempts to supply policy authority

- **WHEN** a request includes a client policy field outside `afc.governed-effect/1`
- **THEN** Gate rejects the request contract rather than silently treating that field as authority

#### Scenario: execution-owner facts are attested

- **WHEN** lease and budget facts arrive from a pinned-issuer authenticated P1 host/UAR caller
- **THEN** Gate validates their identifiers, revisions, active state, and expiry and binds the transport fact source, verified issuer, and authenticated subject to those facts

#### Scenario: caller cannot attest execution-owner facts

- **WHEN** the caller is anonymous, API-key authenticated, or uses an admin provider without a pinned issuer
- **THEN** Gate rejects the authority request before any permit can be returned

### Requirement: durable issuer-scoped approval

Approval challenges SHALL be identified by issuer plus challenge ID and SHALL
record the authenticated administrator that made the decision.

#### Scenario: Postgres is configured

- **WHEN** `approval.backend=postgres` is selected
- **THEN** governed-effect challenges and decisions use Postgres and startup fails if the database is unavailable

#### Scenario: approval is resolved

- **WHEN** an authenticated administrator resolves a live challenge
- **THEN** the decision stores the administrator subject and issuer with the decision timestamp

### Requirement: post-wait revalidation

A resolved approval SHALL NOT itself authorize execution. Gate SHALL revalidate
the exact request and current authority facts after every approval wait.

#### Scenario: approved request is unchanged

- **WHEN** the challenge is approved, unexpired, request binding is identical, authority facts are current, and Cedar still permits or requires approval
- **THEN** Gate returns `permit`

#### Scenario: authority changed during the wait

- **WHEN** payload, resource, identity, policy, grant, lease, budget, approval, or epoch facts changed or were revoked
- **THEN** Gate returns a visible denial and the effect remains unexecuted
