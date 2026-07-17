# approval

## ADDED Requirements

### Requirement: The human-in-the-loop approval flow is verified end-to-end
The gateway SHALL pause a tool call that a policy marks for approval, surface an
approval request, and resume or drop the call on the human decision — and this
behavior SHALL be locked by an end-to-end test so it cannot regress into a silent
deny or allow.

#### Scenario: A required-approval call pauses and surfaces a request
- **WHEN** a tool call evaluates to require approval and an approval handle is
  configured (the normal streaming case)
- **THEN** the call is held (nothing released to the client) and an approval-request
  event is emitted on the stream

#### Scenario: Approve resumes the held call
- **WHEN** the pending approval is approved
- **THEN** the held tool call is released to the client

#### Scenario: Deny drops the held call
- **WHEN** the pending approval is denied
- **THEN** the held tool call is dropped and a deny event is emitted — never
  silently allowed

#### Scenario: No approval handle fails closed
- **WHEN** a required-approval call is reached with no approval handle configured
- **THEN** it is denied (fail-closed) — the documented fallback, distinct from the
  pause-and-resume path above
