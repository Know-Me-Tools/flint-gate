# mcp-authorization

## ADDED Requirements

### Requirement: MCP OAuth 2.1 resource-server authentication
The gateway SHALL act as an OAuth 2.1 resource server for MCP-protected routes, validating inbound bearer access tokens against a configured authorization server's JWKS and binding each token to this resource per RFC 8707.

#### Scenario: Valid audience-bound token is authorized
- **WHEN** a request presents a validly-signed token whose `aud` includes this resource server's configured audience and whose scopes satisfy `required_scopes`
- **THEN** the request is authorized and an `Identity` is derived from the token claims

#### Scenario: Token minted for a different resource is rejected
- **WHEN** a validly-signed token's `aud` does NOT include this resource server's configured audience
- **THEN** the request is rejected with HTTP 401 (RFC 8707 confused-deputy prevention), even though the signature verified

#### Scenario: Token missing required scope is rejected
- **WHEN** a validly-signed, audience-bound token lacks one or more `required_scopes`
- **THEN** the request is rejected with HTTP 403 and `WWW-Authenticate: Bearer error="insufficient_scope"`

#### Scenario: MCP provider without audience or issuer fails closed
- **WHEN** an `mcp` auth provider is configured without `audience` or without `issuer`
- **THEN** the provider is built as a failing authenticator that denies all requests, rather than accepting unbound tokens

### Requirement: Protected resource metadata discovery
The gateway SHALL expose RFC 9728 protected-resource metadata and advertise it on authentication challenges.

#### Scenario: Metadata document is served
- **WHEN** a client requests `.well-known/oauth-protected-resource`
- **THEN** the gateway returns JSON containing `resource`, `authorization_servers`, `scopes_supported`, and `bearer_methods_supported`

#### Scenario: Unauthenticated request advertises metadata
- **WHEN** an MCP-protected route receives a request with no or invalid token
- **THEN** the gateway responds 401 with `WWW-Authenticate: Bearer resource_metadata="<url>"`

### Requirement: Signing-key and transport hardening
The gateway SHALL only trust asymmetric signing keys, pin accepted algorithms, guard JWKS retrieval against SSRF, and never forward the inbound access token upstream.

#### Scenario: Only allowlisted asymmetric algorithms accepted
- **WHEN** a token declares an algorithm outside the RSA/EC allowlist (e.g. HS256 or none)
- **THEN** the token is rejected before key resolution

#### Scenario: Symmetric JWKs and ambiguous kid rejected
- **WHEN** a JWKS contains a symmetric (`oct`) key, or a token omits `kid` against a multi-key JWKS
- **THEN** key selection fails closed and the request is rejected

#### Scenario: JWKS URL SSRF is blocked
- **WHEN** a `jwks_url` targets a loopback, link-local, or private-range host (non-dev), or a non-TLS scheme for a non-loopback host
- **THEN** the provider fails to construct and denies all requests; JWKS fetches follow no redirects

#### Scenario: Inbound token is not forwarded upstream
- **WHEN** an MCP-authenticated request is proxied to the upstream
- **THEN** the inbound `Authorization` bearer token is stripped and not passed through (confused-deputy prevention); a downstream credential is attached only via an explicit claims-enhancement hook
