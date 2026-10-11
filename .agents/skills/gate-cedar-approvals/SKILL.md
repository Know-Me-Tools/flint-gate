---
name: gate-cedar-approvals
description: "Maintain Gate Cedar authorization, tool approval and guardrail contracts with explicit diagnostic and replay handling."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-cedar-approvals

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Own crates/flint-gate-core/src/authz/**, approval/** and guardrail/**. Read schema, bundle, sugar translation, engine, tool_authz and validators together; inspect pipeline and stream consumers without editing their owners' paths.

Cedar is default-deny and forbid overrides permit, but policy evaluation errors are skipped. Never claim Cedar automatically denies every evaluation error. Specify Gate's application-level behavior for diagnostics, missing entities, invalid policy bundles and stale authority; prove the chosen behavior. Validate policies against schemas before publication and keep the last valid version or reject according to the documented contract.

Use structured Cedar entities/values and validated templates, not concatenated untrusted policy text. Bind tenant, principal type, actor/delegation, resource and action. Scope approvals to the exact request/tool arguments or stable digest, policy version, requester and expiry; test replay, concurrent consumption and revocation. Human approval does not create unlimited authority.

Coordinate authority read-model readiness/fences with the control-plane owner and pre-disclosure stream enforcement with the stream owner. Produce allow/deny/error/stale/replay matrices, durable approval evidence and narrowly scoped regression tests. Do not infer policy enforcement from a team manifest's owns list.

## Primary references

- [cedar](https://docs.cedarpolicy.com/auth/authorization.html)
- [cedarSecurity](https://docs.cedarpolicy.com/other/security.html)
