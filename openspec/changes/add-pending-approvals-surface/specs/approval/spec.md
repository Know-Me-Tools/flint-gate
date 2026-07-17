# approval

## ADDED Requirements

### Requirement: Pending approvals are listable by operators
The gateway SHALL expose the set of pending human approvals to operators over the
private admin surface, so a human can see what is awaiting a decision rather than
needing an out-of-band approval id.

#### Scenario: List returns pending approvals
- **WHEN** an operator queries the admin `GET /approvals` endpoint
- **THEN** the currently-pending approvals are returned (id, principal, action,
  resource/tool, expiry), skipping any that have already expired

#### Scenario: Single approval status
- **WHEN** an operator queries `GET /approvals/{id}`
- **THEN** that approval's status is returned, or 404 if it is not pending

#### Scenario: Approval endpoints are private
- **WHEN** the gateway starts with default configuration
- **THEN** the approval list and status endpoints are served only on the
  loopback-bound admin router, never on the public proxy

### Requirement: Operators can decide approvals from the admin UI
The gateway's admin UI SHALL present pending approvals and let an operator approve
or deny each, so the human-in-the-loop flow is usable without hand-crafting API
calls.

#### Scenario: Decide from the UI
- **WHEN** an operator approves or denies a pending approval in the Approvals UI
- **THEN** the decision is posted to the decision endpoint and the paused tool call
  resumes (allow) or is dropped (deny)

#### Scenario: The list reflects local-replica pending approvals
- **WHEN** the gateway runs as multiple replicas
- **THEN** the list and decision surface reflect the pending approvals of the
  replica that serves the request (a documented single-replica constraint; shared
  cross-replica routing is a follow-up)
