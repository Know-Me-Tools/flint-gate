---
name: gate-security-review
description: "Independently test Gate identity, authorization, proxy and stream boundaries and document compatibility gaps with reproducible evidence."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-security-review

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Read broadly; write only assigned tests and docs/agent-teams/flint-gate/reviews/**. Do not silently fix another role's owned code. Evaluate the implementation, not the role's confidence or graph edge counts.

Select attack cases from the change: cross-issuer/tenant confusion, forged actor/agent kind, stale/replayed approval, unknown keys, admin exposure, SSRF/redirects, route encoding, Cedar diagnostic errors, stale authority, stream disclosure before approval and quota races. Use local fixtures and authorized test identities, not real customer tokens or external tenants.

Compare named/versioned Kong/Oathkeeper contracts only for claimed compatibility surfaces. Check both successful operation and explicit rejection/failure behavior; record test commands, observed outputs and source locations. Distinguish confirmed defect from design risk or untested claim.

The implementer cannot serve as its own independent reviewer for identity/policy boundary changes. Report severity, realistic preconditions, reproduction and a bounded remedy to the path owner. Approve only evidence-backed acceptance criteria and retain unresolved limitations in the handoff.

## Primary references

- [jwt](https://www.rfc-editor.org/info/rfc8725/)
- [oauth](https://www.rfc-editor.org/info/rfc9700/)
- [cedar](https://docs.cedarpolicy.com/auth/authorization.html)
- [ory](https://www.ory.sh/docs/oathkeeper/pipeline)
- [ssrf](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
