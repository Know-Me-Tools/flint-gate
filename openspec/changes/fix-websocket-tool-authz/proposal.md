# fix-websocket-tool-authz

**Phase:** beta-release-readiness / Phase 2 (Serious gap S-2)

## Problem

`ws_bridge` in `stream/websocket.rs` does not accept `tool_authz` or
`ApprovalManager` arguments. Any tool call arriving via a WebSocket upstream
bypasses Cedar authorization and the human-in-the-loop approval gate entirely.
The SSE path enforces both; the WS path enforces neither.

## Solution

Port the tool-authz + approval wiring from `SseStreamProcessor` to `ws_bridge`:

1. Add `tool_authz: Option<ToolAuthzContext>` and `approval_handle: Option<ApprovalHandle>` to `ws_bridge`'s signature
2. When a WS text frame contains a `TOOL_CALL_START` event (same JSON shape as SSE), run Cedar evaluation via `tool_authz`
3. On `RequireApproval`, register with `ApprovalManager` and block the frame until a decision arrives (or TTL expires → deny, drop stream)
4. Wire these from `middleware/pipeline.rs` at the WebSocket branch using the same authz context construction as the SSE branch

## Files to change

- `crates/flint-gate-core/src/stream/websocket.rs`
- `crates/flint-gate-core/src/middleware/pipeline.rs`

## Constraints

- The approval path in WS must be fail-closed (same as SSE)
- The TTL auto-deny behavior must be identical to the SSE processor
