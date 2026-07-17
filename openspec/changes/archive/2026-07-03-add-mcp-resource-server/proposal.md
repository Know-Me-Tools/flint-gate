# add-mcp-resource-server

## Summary
Make flint-gate a credible MCP-era gateway by implementing the OAuth 2.1 resource-server surface for MCP: metadata discovery, token audience validation, PKCE, and step-up. (Goal G1)

## Design
Add an `Mcp` auth provider that validates OAuth 2.1 access tokens. Serve RFC 9728 Protected Resource Metadata at `.well-known/oauth-protected-resource` (`axum::Json`); emit `WWW-Authenticate: Bearer resource_metadata="…"` on 401 and `error="insufficient_scope"` on 403 step-up. Validate RFC 8707 `resource`/audience + scopes on decoded claims; verify PKCE S256 (`base64url(SHA256(verifier)) == challenge`) via `sha2`. JWKS discovery/rotation: a cached `HashMap<kid, DecodingKey>` refreshed on unknown `kid` (honor `Cache-Control`), built on the existing `jsonwebtoken@9`. Prevent token passthrough to upstreams (confused-deputy guard).

Library: hand-roll the RS/metadata surface; reuse `jsonwebtoken@9`, `sha2`, `reqwest` (library-candidates.json G1). Do NOT add the `jwks` crate (pins jsonwebtoken ^10, conflicts). Do NOT add `oauth2`/`rmcp` auth (client-only).

## Depends on
- (none — but its identity claims are consumed by add-policy-engine / add-per-tool-authz)

## Scope
IN: RFC 9728 metadata, WWW-Authenticate discovery, RFC 8414/OIDC AS discovery, PKCE S256 verify, RFC 8707 audience/scope validation, 403 step-up, JWKS rotation, no-token-passthrough. OUT: acting as an OAuth2 authorization server; RFC 8693 token-exchange/delegation (deferred to next phase).

## Tasks
- [ ] Add `Mcp` variant to `AuthProviderConfig` + config plumbing
- [ ] Implement `.well-known/oauth-protected-resource` (RFC 9728) JSON handler
- [ ] Emit `WWW-Authenticate: Bearer resource_metadata=…` on 401; `insufficient_scope` on 403 step-up
- [ ] JWKS discovery + rotation cache (`HashMap<kid, DecodingKey>`) on existing jsonwebtoken@9
- [ ] Access-token validation: signature + RFC 8707 audience/resource + scope checks
- [ ] PKCE S256 verification via sha2
- [ ] Enforce no token passthrough to upstream (confused-deputy guard)
- [ ] Tests incl. an MCP client handshake end-to-end; security-review the auth path; ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
