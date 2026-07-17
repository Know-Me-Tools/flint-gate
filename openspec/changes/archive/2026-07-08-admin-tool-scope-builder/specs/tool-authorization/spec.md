# tool-authorization

## ADDED Requirements

### Requirement: Admin tool-scope authoring endpoint
The gateway SHALL expose an admin endpoint that lets an operator author per-agent
tool-scopes structurally (`{ agent, allow[], deny[] }`) and have them compiled to
validated Cedar and enforced, without hand-writing raw Cedar.

#### Scenario: A valid tool-scope is compiled, stored, and activated
- **WHEN** an operator posts a well-formed `{ agent, allow, deny }` entry to the
  admin tool-scope endpoint
- **THEN** it is compiled via the same allowlist-charset `compile_and_validate`
  gate, persisted as the sugar-overlay source, and the live engine is reloaded to
  enforce it

#### Scenario: An illegal agent id or tool token is rejected
- **WHEN** the posted entry contains an agent id or tool token outside the safe
  identifier set
- **THEN** the endpoint rejects it with 400 and nothing is stored (fail-closed —
  the Cedar string-concatenation injection surface is never reached)

#### Scenario: Only the structured path exists for tool-scopes
- **WHEN** an operator authors an agent tool-scope
- **THEN** the only route is the structured `{ agent, allow, deny }` builder that
  compiles through the validator — there is no raw-Cedar bypass for tool-scopes

#### Scenario: Deny still wins from the builder
- **WHEN** a builder entry lists a tool in both `allow` and `deny` (or a `deny`
  glob matches an allowed tool)
- **THEN** the compiled policy denies the call (Cedar `forbid` overrides `permit`)

### Requirement: Admin tool-scope endpoint is private
The admin tool-scope endpoint SHALL be served only on the private admin router,
consistent with the other policy-management endpoints.

#### Scenario: Not exposed on the public proxy
- **WHEN** the gateway starts with default configuration
- **THEN** the tool-scope endpoint is reachable only on the loopback-bound admin
  router, never on the public proxy
