# authorization-audit Specification

## Purpose
TBD - created by archiving change add-authz-audit-trail. Update Purpose after archive.
## Requirements
### Requirement: Authorization decision audit trail
The gateway SHALL record every authorization decision to a durable audit store without blocking or failing the request being decided.

#### Scenario: A decision is recorded
- **WHEN** an authorization decision is made (route-level allow or deny, an MCP insufficient-scope step-up, a per-tool denial, or a future approval outcome)
- **THEN** an audit record is written asynchronously with the principal, action, resource, decision, optional reason, optional context, request id, and timestamp

#### Scenario: Audit writes never block the request
- **WHEN** the audit store is slow or unavailable
- **THEN** the request proceeds on its already-made authorization decision; the audit write is best-effort and its failure is logged, never surfaced to the caller

#### Scenario: No database configured
- **WHEN** the gateway runs without a database
- **THEN** audit recording is a no-op and authorization continues to function

### Requirement: Audit query endpoint
The gateway SHALL expose a read-only, paged, filterable audit query on the private admin surface.

#### Scenario: Filtered, paged read
- **WHEN** an operator queries the admin `/audit` endpoint with optional `principal`, `decision`, `since`/`until`, `limit`, and `offset`
- **THEN** matching audit rows are returned newest-first, with `limit` clamped to a safe maximum and `offset` floored at zero

#### Scenario: Invalid filter is rejected
- **WHEN** the `decision` filter is not a recognized decision value
- **THEN** the request is rejected with 400 rather than silently returning an empty result

#### Scenario: Audit endpoint is private
- **WHEN** the gateway starts with default configuration
- **THEN** the audit endpoint is served only on the loopback-bound admin router, not the public proxy

