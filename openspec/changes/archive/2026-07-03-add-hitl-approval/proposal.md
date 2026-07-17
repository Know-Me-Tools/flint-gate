# add-hitl-approval

## Summary
Pause a flagged agent tool call, request human approval over the existing AG-UI/A2UI stream, and resume or abort on the decision. Deterministic pre-action authorization — the phase's key differentiator. (Goal G5)

## Design
Extend the authz result so a Cedar decision can yield `RequireApproval` (not just Allow/Deny). On `RequireApproval`, pause the tool call and emit an approval-request event over the existing AG-UI/A2UI stream; await an approve/deny decision on a decision channel; resume the tool call or abort. Short-lived approval state store (in-memory + Redis when `redis-l2`, else Postgres). **Decision channel: in-band via AG-UI/A2UI** (leverages the streaming moat) — resolve the exact resume mechanism in this change's design spike; keep an Admin-API decision endpoint as the fallback channel.

Library: none (in-band stream resume).

## Depends on
- add-per-tool-authz (a per-tool decision can require approval)

## Scope
IN: RequireApproval decision, pause/resume of a tool call, approval-request event over AG-UI/A2UI, decision channel, approval state store, end-to-end example. OUT: bundled approval UI beyond the event (the web UI change may add a richer view later).

## Open questions (resolve in this change's design spike)
- In-band stream resume vs out-of-band Admin callback (lean in-band).

## Tasks
- [ ] Design spike: in-band resume mechanism over AG-UI/A2UI (document decision)
- [ ] Extend authz result with `RequireApproval`
- [ ] Emit approval-request event over AG-UI/A2UI; pause the tool call
- [ ] Decision channel (approve/deny) + short-lived approval state store
- [ ] Resume or abort the paused tool call on decision
- [ ] End-to-end example proving pause → approve → resume and pause → deny → abort
- [ ] Tests ≥80% coverage; `cargo check/clippy/test --workspace` green
