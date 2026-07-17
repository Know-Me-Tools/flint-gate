# approval

## ADDED Requirements

### Requirement: An undecided approval fails closed
The gateway SHALL bound the time a tool call is paused awaiting a human approval:
when the approval's TTL elapses with no decision, the held call is **denied**
(never left pending indefinitely, never silently allowed), and the stream resumes
to a clean termination.

#### Scenario: An undecided approval times out to deny
- **WHEN** a tool call is paused for approval and no decision arrives before the
  approval's `expires_at`
- **THEN** the held call is resolved as Deny — the deny event is emitted and the
  stream terminates cleanly, with no half-open stream and no silent allow

#### Scenario: Staggered approvals each deny at their own deadline
- **WHEN** two approvals are pending with different expiry times and neither is
  decided
- **THEN** each is denied at its own deadline (the deadline is the nearest pending
  expiry, recomputed as each is resolved)

#### Scenario: A decision before the deadline still wins
- **WHEN** a decision arrives before the TTL elapses
- **THEN** the call resumes (allow) or is dropped (deny) per the decision, and the
  timeout does not fire

### Requirement: Expired approvals are reaped
The gateway SHALL periodically purge expired pending approvals from the manager,
so entries whose streams have already ended do not accumulate.

#### Scenario: The janitor reaps expired entries
- **WHEN** a pending approval has passed its expiry and the background janitor runs
- **THEN** the entry is removed from the pending-approval store

### Requirement: Approval is operator-configurable
The gateway SHALL expose approval TTL and an enable flag in configuration, with a
fail-closed disable.

#### Scenario: TTL override
- **WHEN** `approval.ttl_seconds` is set
- **THEN** pending approvals use that TTL instead of the built-in default

#### Scenario: Disabling approvals fails closed
- **WHEN** `approval.enabled` is false and a policy evaluates to require approval
- **THEN** the tool call is denied (not paused, not allowed) — an operator who
  cannot service approvals denies rather than hangs

#### Scenario: Default is enabled
- **WHEN** no `approval` config is present
- **THEN** approvals are enabled with the built-in default TTL (behavior unchanged)
