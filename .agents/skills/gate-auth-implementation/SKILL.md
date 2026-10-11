---
name: gate-auth-implementation
description: "Implement and review Gate JWT verification/minting, API keys, Kratos sessions, OAuth/PKCE and constrained token exchange."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-auth-implementation

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Own crates/flint-gate-core/src/auth/**. Follow the identity architect's issuer/subject/tenant/actor contract. Trace JwtVerifyAuthenticator::new -> JwksCache::new and exchange -> build_delegated_claims; structural edges do not establish correct trust decisions.

Inspect every Authenticator implementation and the pipeline consumer before changing Identity. Preserve partition_jwt_claims and strip_untrusted_kind intent: untrusted external claims or Kratos metadata cannot self-assert gateway agent authority. Validate credential ambiguity and authoritative session expiration.

Use strict token-purpose validation, bounded JWKS refresh, timeout/body-size limits on outbound identity calls, and key overlap during rotation. Never trust jku/x5u/issuer URLs supplied by a token without configured trust. Do not log credentials. For OAuth examine redirect matching, state/PKCE binding, client authentication, refresh replay and token exchange actor depth/scope/audience reduction. Preserve the distinction between local exchange and delegation to Hydra. Do not authorize impersonation from an arbitrary act claim.

Prove wrong issuer/audience/algorithm, expired/not-yet-valid token, unknown/rotated key, ambiguous credentials, forged agent kind, delegated scope escalation and upstream identity outage behavior with relevant tests. Ask the policy owner to review authorization effects; the auth implementer cannot self-approve a security boundary.

## Primary references

- [jwt](https://www.rfc-editor.org/info/rfc8725/)
- [oauth](https://www.rfc-editor.org/info/rfc9700/)
- [kratos](https://www.ory.sh/docs/kratos/guides/zero-trust-iap-proxy-identity-access-proxy)
- [okta](https://developer.okta.com/docs/guides/validate-access-tokens/dotnet/main/)
- [ssrf](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
