---
name: gate-proxy-security
description: "Maintain Gate route matching and HTTP trust boundaries with measured Kong/Oathkeeper compatibility and SSRF protection."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-proxy-security

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Own crates/flint-gate-core/src/proxy/**, middleware/** and crates/flint-gate/src/**. Read main composition and handle_request end-to-end: route selection, authentication, claims/body hooks, authorization and upstream forwarding.

Compare a named/versioned Kong Route/plugin or Oathkeeper access-rule behavior using observable fixtures. Do not promise global drop-in compatibility or repeat stale README vendor limitations. Oathkeeper separates matching, authentication, authorization and mutation; Gate order and failure semantics must be explicit.

Test host/path/method priority, normalization and encoded paths, untrusted forwarding headers, internal identity-header stripping, body-size limits, redirects, upstream URL allowlists, DNS/address handling and connection timeouts. Request hooks must not silently widen identity or privilege. Prevent admin routes/port 4457 from leaking onto proxy 4456. Keep configured private upstreams possible without accepting arbitrary client-directed destinations.

Trace request cancellation, backpressure and upgrades with the streaming owner. Hand off authenticated-principal and authorization changes rather than duplicating their implementations. Return route truth tables and tests for bypass attempts as well as ordinary routing.

## Primary references

- [ory](https://www.ory.sh/docs/oathkeeper/pipeline)
- [kong](https://developer.konghq.com/gateway/entities/route/)
- [plugins](https://developer.konghq.com/gateway/entities/plugin/)
- [ssrf](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
