---
name: gate-client-security
description: "Maintain Flint Gate admin UI and Rust/TypeScript/Go/Flutter client contracts without weakening server-side authorization."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-client-security

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Read the actual admin/proxy route, credential, error and streaming contracts before changing clients. Console owner writes web/src and web configuration; SDK owner writes flint-gate-client and sdks. Respect those separate assignments.

Keep bearer/API keys out of URLs, error messages, analytics and checked-in fixtures. Browser session/cookie flows need explicit CSRF, SameSite and origin decisions; CORS is not authorization. Escape untrusted policy, identity and event data in the UI. Explain denied, expired, pending approval and stale authority states accurately; hiding a button is not an access check.

For Rust, TypeScript, Go and Dart preserve status/error typing, cancellation, stream parsing, retry bounds and idempotency semantics. Do not automatically retry a possibly committed mutation or replay a credential to a different host. Test supported endpoint/version contracts and cross-language payloads. Coordinate model-changing admin responses with the control-plane owner.

Deliver accessible operator controls, contract fixtures and redacted examples; document which server features each SDK exposes and remaining gaps rather than claiming feature parity by package presence.

## Primary references

- [oauth](https://www.rfc-editor.org/info/rfc9700/)
- [jwt](https://www.rfc-editor.org/info/rfc8725/)
