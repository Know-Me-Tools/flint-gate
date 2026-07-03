# tool-authorization Specification

## Purpose
TBD - created by archiving change add-per-tool-authz. Update Purpose after archive.
## Requirements
### Requirement: Per-tool-call authorization with inspect-then-forward
The gateway SHALL authorize each agent tool call against the policy engine on its COMPLETE arguments before any part of the call is forwarded to the client, buffering the tool-call events until the decision resolves.

#### Scenario: Authorized tool call is released after full-args check
- **WHEN** an AG-UI tool call (`TOOL_CALL_START` → `TOOL_CALL_ARGS`* → `TOOL_CALL_END`) is authorized on its complete accumulated arguments at END
- **THEN** the held START, a single coalesced ARGS event carrying the full arguments, and END are flushed downstream in order

#### Scenario: Denied tool call forwards no arguments
- **WHEN** a tool call is denied (at the coarse name check on START, or the fine arguments check on END)
- **THEN** no `TOOL_CALL_ARGS` bytes reach the client, and only a synthetic `RUN_ERROR` is emitted for that tool-call id

#### Scenario: Arguments are not streamed before authorization
- **WHEN** `TOOL_CALL_ARGS` deltas arrive for a not-yet-authorized call
- **THEN** they are buffered and not forwarded; the client observes the arguments only after the complete call is authorized at END

#### Scenario: Uncorrelated or malformed tool events fail closed
- **WHEN** a `TOOL_CALL_ARGS`/`TOOL_CALL_END` arrives with no preceding START, an unknown id, a missing tool name, or arguments that are non-empty but unparseable
- **THEN** the events are dropped or the call is denied — never forwarded

#### Scenario: Non-tool events stream live
- **WHEN** the stream carries `TEXT_MESSAGE_CONTENT` or other non-tool events (with or without an authorization context)
- **THEN** they stream through live with no added buffering latency

#### Scenario: Routes without an authorization context are unaffected
- **WHEN** a route has no tool-authorization context configured
- **THEN** tool-call events stream through one-for-one exactly as before this capability existed

### Requirement: Tool listing visibility filtering
The gateway SHALL remove unauthorized tools from tool-listing responses, failing closed when a listing cannot be parsed.

#### Scenario: Unauthorized tools are hidden
- **WHEN** an MCP `tools/list` response passes through and some listed tools evaluate to Deny
- **THEN** those tools are removed from the response before it is forwarded

#### Scenario: Malformed listing fails closed
- **WHEN** a message is recognized as a `tools/list` response but its tools payload cannot be parsed as expected (including JSON-RPC batch forms)
- **THEN** the tools array is stripped rather than forwarded intact, so no unauthorized tool leaks

### Requirement: Streaming resource bounds
The gateway SHALL bound the memory used to buffer streaming events and tool-call arguments.

#### Scenario: Oversized event terminates the stream
- **WHEN** buffered event/line data exceeds the configured maximum
- **THEN** the stream is terminated (fail-closed)

#### Scenario: Oversized tool arguments deny the call
- **WHEN** a single tool call's accumulated arguments exceed the configured maximum
- **THEN** that tool call is denied without tearing down the rest of the stream

