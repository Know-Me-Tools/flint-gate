# Refinement Log — add-per-tool-authz (QA + security + research gate)

**Date:** 2026-07-03 · phase agent-authz-control-plane · change 4/8

## Security review found 1 CRITICAL + 2 HIGH + fail-open edge (L2)
- C1 (CRITICAL): unbounded event/args buffering DoS. FIXED: DEFAULT_MAX_EVENT_BYTES (256KB), DEFAULT_MAX_TOOL_ARGS_BYTES (1MB), overridable via BackpressureConfig; oversized event terminates stream, oversized args denies call.
- H1 (HIGH): forward-then-annul (args streamed before END re-check; omitting END skipped the fine gate). RESOLVED via deep protocol + industry research → reworked to BUFFER-UNTIL-AUTHORIZED (inspect-then-forward): START/ARGS held, authorized on complete args at END, flushed only on allow; RUN_ERROR + zero args on deny.
- H2 (HIGH): list_tools filter failed OPEN on unknown shapes. FIXED: fail-closed — malformed tools/list responses have tools stripped; JSON-RPC batch handled.
- L2: unparseable args coerced to {} (fail-open). FIXED: non-empty unparseable args → Deny.

## Research basis for H1 (2 firecrawl studies, ~15 sources)
- AG-UI spec: clients execute a tool ONLY after TOOL_CALL_END; args deltas are display-only. Blocking END is a real execution control; buffering args additionally closes info-disclosure + non-conformant-consumer gaps. Tool-arg latency is low-stakes (unlike TEXT_MESSAGE_CONTENT).
- Industry (agentgateway request-time mcp.tool.arguments; Kong tools/list filtering; Portkey/TrueFoundry "streaming output = informational only unless you buffer"; LangGraph interrupt-before-execute): universal inspect-then-forward; forward-then-annul is not a real control. MCP tools/call is atomic (args never partial) — partial-args is an AG-UI-only artifact.
- Decision (user-confirmed): buffer-until-authorized + fix C1/H2/L2 in this change.

## Independent verification (orchestrator)
- Caught a transient mid-edit build break (stale `process` callers) — waited for the agent to finish; final tree builds clean.
- Ran 11 security-critical tests individually — all pass: buffer-holds-until-end, denied-forwards-no-args, no-start-leak, args-without-start dropped, oversized-args deny, event-cap terminate, malformed-listing stripped, unparseable-args deny, text streams live, non-authz routes unaffected.
- cargo test --workspace: 256 pass (240 core lib), 0 failed, 3 ignored. clippy clean both feature sets. fmt clean. No prod unwrap/expect.

## Constraint checks
| Constraint | Result |
|-----------|--------|
| No unwrap/expect outside tests | PASS |
| No secrets | PASS |
| Fail-closed authz | PASS (every error/ambiguity → deny/drop) |
| Existing tests not broken | PASS (text/non-tool/backward-compat all green) |

## Gates
- openspec validate --strict → valid (delta spec: specs/tool-authorization)
- clippy --workspace --all-features / -p core --no-default-features → clean
- cargo test --workspace → 256 pass, 0 failed, 3 ignored

## Verdict: PASS — cleared to archive.
