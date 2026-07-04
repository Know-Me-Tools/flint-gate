---
type: Reference
id: flint-gate-agent-authz-control-plane-checkpoint
title: Flint Gate agent authz control plane checkpoint
tags:
- flint-gate
- agent-authz
- control-plane
- mcp-resource-server
- oauth-2-1
- jwks
- pkce
- security-review
links:
- agent-authorization-control-plane-executor-session
sources:
- stdin
- manual:flint-gate/agent-authz-control-plane
- .kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md
timestamp: 2026-07-03T16:33:36.850177+00:00
created_at: 2026-07-03T16:33:36.850177+00:00
updated_at: 2026-07-03T16:33:36.850177+00:00
revision: 0
---

## Context

- **Project:** `flint-gate`
- **Phase:** `agent-authz-control-plane`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/flint-gate`
- **Captured:** `2026-07-03T16:27:09Z`
- **Status:** `execution_ready`
- **Progress:** change `1/8` complete; change 2 is `7/9` tasks complete with security fixes in flight

This phase turns `flint-gate` from an auth proxy into an MCP-era agent gateway by adding an agent-authorization control plane on top of existing streaming enforcement:

- Mid-stream SSE token metering
- Session watchdog
- AG-UI/A2UI processing

Scope is authorization-first. The phase deliberately excludes off-identity LLM-ops features such as semantic caching, multi-LLM routing, and multimodal routing.

This checkpoint adds implementation detail beyond earlier sparse executor records such as [Agent Authorization Control Plane Executor Session](/agent-authorization-control-plane-executor-session.md).

## Build Order and Goals

### 1. Budget enforcement and windowed rate limiting

Extend existing `usage_events` plus lifetime `MaxTokenBudget` hook into:

- Per-key and per-team rolling-window token budgets
- Minute/hour/day token windows
- Request-rate limits
- Threshold blocking with clear errors

Rationale: closes gap `G3`; fastest win and highest-feasibility control-plane improvement.

### 2. MCP OAuth 2.1 resource-server support

Implement MCP resource-server support as the credibility gate for the gateway:

- RFC 9728 protected-resource metadata
- `WWW-Authenticate: resource_metadata` on `401`
- RFC 8414 / OIDC authorization-server discovery
- PKCE S256 verification
- RFC 8707 `resource` / audience validation
- `403 insufficient_scope` step-up behavior
- No token passthrough to upstreams to prevent confused-deputy replay

Rationale: closes gap `G1`; critical security requirement for MCP-era identity.

### 3. Embedded policy engine and per-tool-call authorization

Add inline authorization for MCP tool calls using an embedded native Rust policy engine:

- Candidate engines: Cedar core or `casbin-rs`
- No sidecar dependency
- New `PreRequestHook::Authorize`
- Stream-level tool-call gate
- Authorize each MCP tool call by:
  - Tool name
  - Parameters
  - Identity claims
- Filter unauthorized tools out of `list_tools` responses, following the agentgateway pattern

Rationale: closes gap `G2`; strategic core of the agent gateway.

## Current Session Progress

### Completed

- Committed change 1 to the feature branch:
  - Commit: `e4b871b`
- Delegated MCP resource-server tasks 1–7 to the Rust agent.
- Rust agent delivered a clean implementation with:
  - `auth/mcp.rs`
  - `mcp_metadata.rs`
  - `jwks.rs`
  - `pkce.rs`
  - Shared `JwksCache` refactor
  - RFC 8707 audience binding
  - No-token-passthrough guard
- Core tests passing at handoff:
  - `135` tests passed

### Independent Security Inspection

The security crux was manually inspected. A bypass was found:

- `audience: None` could bypass RFC 8707 audience binding and create a confused-deputy token replay path.

### Security Reviewer Findings

The security-reviewer agent was run as required for authorization-sensitive changes. It confirmed the two security-defining happy-path requirements:

- RFC 8707 audience enforcement
- No-token-passthrough protection

It also found six hardening gaps:

| Severity | ID | Finding |
| --- | --- | --- |
| Critical | `C1` | `audience: None` permits confused-deputy bypass |
| High | `H1` | JWKS SSRF via unvalidated `jwks_url` |
| High | `H2` | `kid: None` / symmetric-JWK downgrade risk |
| Medium | `M1` | Missing JWT algorithm allowlist |
| Medium | `M2` | JWKS unknown-`kid` refresh amplification |
| Medium | `M3` | `issuer` not required |

All six fixes plus an end-to-end WireMock handshake test were dispatched back to the Rust agent and were running at capture time.

## Required Verification Before Change 2 Closure

When the fix agent returns:

1. Independently verify all six security findings are resolved:
   - `C1` audience must be mandatory where required; `audience: None` must not bypass resource/audience validation.
   - `H1` JWKS URLs must be validated to prevent SSRF.
   - `H2` tokens without `kid` and symmetric/downgrade-prone JWK paths must be rejected or safely constrained.
   - `M1` JWT algorithms must be explicitly allowlisted.
   - `M2` unknown-`kid` refresh behavior must be bounded to prevent amplification.
   - `M3` issuer must be required and validated.
2. Confirm all tests pass, including the WireMock end-to-end MCP OAuth handshake test.
3. Close MCP resource-server tasks 8–9.
4. Run the QA gate.
5. Add a delta spec.
6. Run strict `openspec validate`.
7. Archive change 2.
8. Commit the completed change.

## Remaining Phase Work

- Finish change 2: MCP OAuth 2.1 resource-server support with security fixes.
- Complete changes 3–8.
- Reflect after phase completion.

# Citations

1. stdin
2. manual:flint-gate/agent-authz-control-plane
3. .kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md