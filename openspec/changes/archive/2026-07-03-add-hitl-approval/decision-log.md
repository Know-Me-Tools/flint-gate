# add-hitl-approval Design Decision Log

## Decision: In-band AG-UI/A2UI stream resume for human approval

### Context
Goal G5 of the `agent-authz-control-plane` phase requires deterministic, pre-action
authorization for agent tool calls. When a Cedar policy evaluates to
`RequireApproval`, the gate must pause the flagged tool call, request human
approval, and then either resume the call or abort it. The proposal left the
exact resume mechanism as an open question to be resolved in this change's
design spike.

### Decision
Use **in-band resume over the existing AG-UI/A2UI SSE stream** as the primary
decision channel. Approval is requested by emitting a synthetic approval-request
event into the same downstream SSE stream that carries the paused tool call. The
human operator (or a frontend acting on their behalf) responds by sending a
matching approve/deny decision event back through the same stream connection.

An Admin API decision endpoint will be retained as a fallback channel for
headless or out-of-band scenarios, but it is not the default path.

### Rationale
- **Leverages the streaming moat.** The stream processor already buffers tool
calls (`TOOL_CALL_START`/`ARGS`/`END`) for per-tool authorization, so pausing at
`TOOL_CALL_END` requires no new connection lifecycle.
- **Fails closed by default.** Withholding `TOOL_CALL_END` from the downstream
client keeps the call un-executed while approval is pending.
- **No extra connection plumbing.** Reusing the SSE stream avoids WebSocket
upgrade complexity, separate approval sockets, or out-of-band callback
coordination in the common case.
- **Protocol-agnostic shape.** Both AG-UI and A2UI processors can emit a
synthetic event with a well-known `type`/`intent`, so the same mechanism serves
both protocols.
- **Fits existing architecture.** `AuthzDecision` is already evaluated inside
`AgUiProcessor`/`A2UiProcessor`; extending it with `RequireApproval` keeps the
authorization surface small and localized.

### Consequences
- `AuthzDecision` gains a `RequireApproval` variant carrying an opaque
approval-context payload (tool name, call id, reason/policy trace, expiry).
- The AG-UI/A2UI processors must track paused tool calls keyed by a stable call
id, and hold back the buffered `TOOL_CALL_END` until a decision arrives.
- New event types/intents are introduced:
  - `gate:approval_request` (downstream) carrying `approval_id`, `tool_name`,
    `arguments_summary`, `expires_at`.
  - `gate:approval_decision` (upstream) carrying `approval_id` and
    `decision: "approve" | "deny"`.
- A short-lived approval state store is required to correlate decisions with
paused calls. Default is in-memory; Redis is used when the `redis-l2` feature is
enabled; otherwise Postgres.
- The stream processor's `process_chunk` return type may need to signal a
"paused awaiting decision" state so the proxy task knows to stop reading from
upstream until the decision arrives.
- Decision events must be filtered out of the downstream client stream and
consumed only by the gate.

### Alternatives considered
- **Out-of-band Admin callback / webhook:** Rejected for the primary path
because it complicates deployment and adds external dependency for the common
interactive case. Retained as fallback.
- **Out-of-band Admin API long-polling:** Rejected as primary because it forces
the frontend to manage a second request lifecycle while the SSE stream is
already open.

### Implementation notes
- Approval context should be small and serializable; full tool arguments are NOT
included in the request event to avoid leaking sensitive parameters. Include a
sanitized summary or hash instead.
- Each `approval_id` is a short, collision-resistant identifier (e.g. ULID or
time-ordered UUID) scoped to the request.
- Expiry is enforced both in the state store TTL and by the processor's own
timer; expired approvals resolve to deny.
- The processor remains single-threaded per stream, so paused-call state can
use interior mutability (`RefCell`) like existing tool-call state.

### Open questions resolved
- In-band stream resume vs out-of-band Admin callback: **in-band primary,
Admin API fallback.**
