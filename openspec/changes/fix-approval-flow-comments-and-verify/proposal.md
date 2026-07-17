# fix-approval-flow-comments-and-verify

**Phase:** agent-approval-and-step-up-flows · **Goal:** G1 (build-003)
**Scope:** `crates/flint-gate-core/src/stream/ag_ui.rs` + `stream/a2ui.rs` (comment
fixes), tests (end-to-end verification), docs.
**Depends on:** nothing (independent; ordered last — the flow already works).

## Why

The end-to-end pause → request → resume/deny approval flow is already implemented
and genuinely pauses (Assess). But two comments —
`stream/ag_ui.rs:245` and `stream/a2ui.rs:140` ("`RequireApproval` is treated as a
deny (fail-closed)") — describe only the **no-handle fallback** branch, not the
live behavior. As written they are misleading: a future reader could "correct" the
flow into an actual deny, silently removing the human-in-the-loop opportunity. The
working flow also lacks an explicit end-to-end regression test.

## What

1. **Fix the misleading comments** at `ag_ui.rs:245` and `a2ui.rs:140` to state
   precisely that the fail-closed deny applies **only when no approval handle is
   configured**; with a handle (the normal streaming case) the call pauses and
   awaits a decision.
2. **End-to-end verification test** — a test that drives a `RequireApproval`
   decision through the stream processor and asserts: the call **pauses** (nothing
   released), an approval-request event is emitted, an **approve** resumes/flushes
   the held call, and a **deny** drops it with the deny event. Locks the behavior
   the comments previously obscured.

## Non-goals

- In-band client decision channel (client-decides over the same stream) — OUT of
  scope; the Admin REST decision + the UI (change 2) is the operator flow. A
  larger stream-protocol change, deferred.
- Changing the flow's behavior (it works; this is verify + document).

## Fail-safe requirement

No behavior change — the flow stays fail-closed in the no-handle case and
pause-and-resume in the handle case. The verification test asserts no silent-allow
path. (This change is documentation + test; the timeout auto-deny fail-closed is
covered by `add-approval-timeout-and-janitor`.)

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
the end-to-end pause/approve/deny test passes; existing tests unregressed.
