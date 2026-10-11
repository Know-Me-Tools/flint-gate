---
name: gate-operations
description: "Operate Gate listeners, keys, database/cache dependencies, observability and recovery without exposing its administration plane."
license: MIT
metadata:
  version: "1.0.0"
  source: "Project-authored from inspected source and primary documentation, 2026-09-25"
---

# gate-operations

Environment: Flint Gate source checkout; optional Compass MCP or CLI. No external service access is required to read this skill.

Own deployment manifests, CI/operator scripts, metrics and operator documentation assigned in the team manifest. Read current deployment and configuration types before applying a historical deploy skill. Do not change a running service merely because an example command exists.

Keep 4456 proxy and 4457 administration network boundaries explicit. Use environment/secret-manager references, never committed keys or credentials. Plan staged signing-key rotation with verification overlap and recovery; coordinate JWKS/session caching and emergency revocation with auth/control-plane owners.

Define readiness around authority/bootstrap dependencies, graceful stream draining, timeouts, database migration compatibility, Redis degraded behavior and backup restoration. Multiple replicas need tested shared state/fencing for approvals, quotas and authority. Publish metrics with bounded cardinality and no raw subjects/tokens in labels.

Supply change-specific rollout/rollback checks and failure evidence. Run intended scope tests or build jobs once per coordinated change; one build owner avoids redundant expensive Rust jobs. A graph build does not compile Gate. No production deployment or remote identity administration is implied by this skill.

## Primary references

- [jwt](https://www.rfc-editor.org/info/rfc8725/)
- [cedarSecurity](https://docs.cedarpolicy.com/other/security.html)
