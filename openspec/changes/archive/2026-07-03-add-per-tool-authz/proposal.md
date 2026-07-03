# add-per-tool-authz

## Summary
Authorize each MCP tool call inline in the stream and filter unauthorized tools out of `list_tools`. Turns flint-gate from connection-auth into agent-action-authz. (Goal G2b — strategic core)

## Design
Represent each MCP tool call as one Cedar `Request`: principal = identity (from add-mcp-resource-server), action = generic `call_tool`, resource = the tool, `context` = tool params + `tool_name`. Evaluate inline in `stream/processor.rs`; on `Deny`, block the tool call. For MCP `list_tools` responses, evaluate each candidate tool and drop those that evaluate `Deny` before returning to the client (the agentgateway pattern). Reuse the Cedar bundle + ArcSwap from add-policy-engine (no new engine).

Library: cedar-policy (from add-policy-engine).

## Depends on
- add-policy-engine (Cedar bundle + Authorize hook)
- add-mcp-resource-server (identity claims as principal)

## Scope
IN: per-tool-call authorization in the stream, `list_tools` filtering, generic `call_tool` action modeling. OUT: HITL approval (add-hitl-approval), audit (add-authz-audit-trail).

## Tasks
- [ ] Map an MCP tool call to a Cedar Request (principal/action/resource/context)
- [ ] Inline evaluation in `stream/processor.rs`; block tool call on Deny
- [ ] Filter `Deny` tools out of MCP `list_tools` responses
- [ ] Handle tool-call decisions in `stream/a2ui.rs` where A2UI intents invoke tools
- [ ] Tests: allow/deny tool call, list_tools filtering, param-based decisions; ≥80% coverage
- [ ] `cargo check/clippy/test --workspace` green
