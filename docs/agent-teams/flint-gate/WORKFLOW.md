# Selecting and operating the Gate team

Ask what behavior must change, which caller experiences it, the acceptance examples and the affected deployment mode. Inspect source before asking users to name agent roles. Use the smallest useful assignment:

| Task | Starting assignment | Add when the change crosses the boundary |
|---|---|---|
| Okta/OIDC login or API token validation | identity-architect + auth + reviewer | control-plane for provider settings; console/SDK for exposure |
| DID/VC or human/agent delegation design | identity-architect + policy + reviewer | auth after the trust/presentation contract is defined |
| Kong/Oathkeeper route migration | proxy + reviewer | identity-architect/auth for changed identity contracts |
| Approval, Cedar policy or tenant isolation | policy + reviewer | control-plane for authority/versioning; streams for disclosure |
| Streaming expiry, tool gating or budgets | streams + reviewer | policy/auth/control-plane for cross-boundary contracts |
| Admin configuration or replicated cache | control-plane + reviewer | console/SDK/operations for affected consumers |
| Console interaction or SDK bug | console or SDK | reviewer for identity, retry or authorization behavior |
| Deployment/recovery change | operations | control-plane/auth plus reviewer for state or key changes |

Names in this table abbreviate the `gate-` prefix. These are selectable task compositions, not a requirement to start every listed role. Roles are reusable definitions, not persistent servers.

## Task contract

1. Inspect `git status` and source digests. Reserve task-specific paths within the manifest ownership; identify current writers before editing. Existing uncommitted changes are inputs, not cleanup targets. A new path outside the roster needs a deliberate ownership update; it is not free territory.
2. Reconcile KBD before application execution. At setup, `project.json` selected `post-beta-hardening`, the waypoint referenced `agent-authz-control-plane`, and production-readiness progress reported completed work contradicted by older notes. Do not select a phase merely because one file looks newer. Resolve the canonical project/phase/iteration/task/boundary using the actual KBD tooling and active owner's context; record the resolution without hand-editing generated projections. This setup did not advance any phase.
3. Specify acceptance criteria, trust boundaries, affected consumers and a task DAG. Manifest `dependsOn: []` intentionally defines an on-demand specialist pool. The real task must wait for required producer handoffs before implementation/review; an empty roster dependency list is not permission to review unfinished work or run an entire exported workflow concurrently.
4. Select one execution owner: Codex, Claude, OpenCode, Kimi, MiniMax, Copilot, DeepSeek, UAR or BossFang as configured. Keep native permissions and model controls authoritative. The ledger, prompts and ownership globs do not implement Cedar or filesystem enforcement. Do not start nested competing loops.
5. Bind selected skills and a compatible model. Hard tasks include identity boundaries, policy semantics and independent security review. Routine console/SDK/operations defaults may use medium strength; key rotation, credential storage or security-sensitive retry semantics require hard review. Use current discovery/price data when budget matters; unknown values remain unknown.
6. Implement only assigned paths, route shared edits to the owner, and run focused behavioral checks. Tests should prove observable success and rejection/failure behavior. Coordinate one Rust build writer. Attach actual commands/results, not inferred success from code graph coverage. Relevant existing tests may be modified by their owning role; independent review writes only its own test/findings paths.
7. Review independently, integrate handoffs, and complete canonical KBD boundaries through the actual CLI. Use the existing Karpathy process only at real boundaries, with real receipts. Do not invent a completed task to populate progress logs.

## Cross-harness handoff

Record repository path, branch/HEAD, dirty-file inventory, graph digest/time, canonical task identity, acceptance criteria, exact owner/write scope, dependencies, changed paths, test commands/results, pending failures, model/provider assumptions and next action. Use agent-team-manage/agent-team-handoff when available. A local Markdown/JSON receipt with these fields is the fallback, not an invented remote API call.

Shared memory is optional. Gate identity subjects, session cookies, bearer tokens, private keys, tenant records and customer prompts must not be copied into the team memory stream. Scope/retrieval filters in a shared memory tool do not prove tenant authorization. Publish only redacted evidence to an explicitly configured, authorized endpoint and retain uncertainty/idempotency receipts. Local files remain sufficient when memory services are unavailable.

## Ecosystem and service boundary

Gate owns credential-to-principal and request/stream enforcement at ingress. Forge's PostgreSQL/RLS checks, UAR/BossFang execution permissions and downstream service checks remain separate enforcement points. Agree on issuer/audience/subject/tenant/actor, correlation IDs, policy versions, expiry and event schemas at each hop. Fabric transport does not automatically make its event payloads authoritative; subscriber authorization and revocation are still needed. KnowMe and the-boss require explicit user/agent identity separation. This team may inspect those contracts and write Gate-side ADRs; counterpart changes belong to their repository owners.

UAR/BossFang exports require a selected instance, credential environment reference, native tool permissions and a task-specific composition before registration. Record each returned ID; registration is not atomic. The chosen future BossFang path is standalone agents plus a bounded workflow. The exported Hand is an unused alternative and must not be activated alongside that path. No service calls or activation occurred during setup.
