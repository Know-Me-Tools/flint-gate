# Goals — agent-approval-and-step-up-flows

_Seeded from `agent-governance-completeness-and-policy-authoring/reflection.md` →
"Recommended Next Phase → Option A" (operator-selected). The prior phase-line made
agent governance **complete and self-defending**; this phase deepens the
authorization-first stance into **interactive governance** — the human-in-the-loop
approval / step-up path the Cedar engine already models but does not yet surface
end-to-end._

## Phase Goal

Make **human-in-the-loop tool-call approval** a complete, observable flow: when a
Cedar policy evaluates to `RequireApproval`, the tool call **pauses**, an approval
request is **surfaced** to a human over the streaming channel (AG-UI / A2UI), and
the call **resumes or is denied** on the human's decision — with an operator view
of pending approvals and a fail-closed default (timeout / unavailable → deny).
Still **authorization-first**; still **federate any JWKS-capable IdM (Ory
reference), never an IdP**; the LLM-ops bundle stays out of scope.

**Seeded from:** `agent-governance-completeness-and-policy-authoring` reflection ·
**Criteria profile:** effort-impact

## Known starting point (VERIFY + refine in Assess)

The prior work already built substantial approval primitives — Assess MUST map the
exact current state before specs are written; these are the *observed* pieces, not
a complete inventory:

- `AuthzDecision::RequireApproval(ApprovalContext)` produced by the engine on a
  `@require_approval`-annotated permit (`authz/engine.rs`).
- An `ApprovalManager` (`approval/mod.rs`) with `register` / `decide` / `status` /
  `purge_expired` / `len`, `ApprovalDecision`, `ApprovalError`, `ApprovalStatus`.
- Stream wiring: `stream/ag_ui.rs` and `stream/a2ui.rs` already call
  `request_approval` on the `RequireApproval` path (currently some paths treat it
  as fail-closed deny; a2ui has a "pauses the call" note).
- An admin `POST /approvals/{id}/decision` endpoint (`decide_approval_handler`).

So this phase is likely **completing + surfacing** an in-progress flow, not building
from scratch. Assess decides which of the goals below are already partly met.

## Goals (build order — dependency-aware; refined by Assess/Spec)

1. **End-to-end pause → request → resume/deny over the wire** *(the core flow —
   BUILD/VERIFY FIRST).* Confirm and complete: a `RequireApproval` tool call pauses
   the stream, emits an approval-request event on the AG-UI / A2UI channel with the
   `ApprovalContext` (principal, tool, reason), and resumes (allow) or drops (deny)
   when the decision arrives — no path silently allows. Any gap where
   `RequireApproval` is treated as a plain deny (losing the human-in-the-loop
   opportunity) is closed. (HIGH.)

2. **Pending-approvals operator surface** *(admin API + UI).* A read-only admin
   endpoint listing pending approvals (id, principal, tool, requested-at, expiry)
   and the ability to decide them from the admin UI — mirroring the existing
   Policies / Agent-identities pages. Makes the `ApprovalManager` state operable,
   not just log-visible. (MEDIUM.)

3. **Fail-closed lifecycle: timeout, expiry, and unavailability** *(the safety
   floor).* A pending approval that is never decided must **time out to deny**
   (bounded, configurable), `purge_expired` must run, and an approval channel /
   store that is unavailable must **fail closed (deny)** — a tool call must never
   hang forever or silently proceed. Test every path. (MEDIUM — the fail-safe
   close of the interactive flow.)

## Explicitly out of scope (this phase)

- The LLM-ops bundle (semantic caching, multi-LLM routing/LB, prompt compression,
  multimodal, prompt versioning) — off-identity.
- Becoming a full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.
- Multi-approver / quorum approval and approval delegation policies (single-decision
  human-in-the-loop this phase; quorum is a later extension).

## Carried-over open questions (resolve during Assess/Analyze)

- **Actual coverage:** which of goals 1–3 are already (partly) implemented? Assess
  must trace the `RequireApproval` path through ag_ui/a2ui + `ApprovalManager` +
  the decision endpoint and report what's missing vs. present.
- **Event schema on the wire:** what AG-UI / A2UI event type carries an approval
  request, and does it fit the existing `allowed_events` / `allowed_intents`
  validation? (Crux of goal 1.)
- **Timeout model:** where does the pause live (a per-call awaited channel with a
  deadline?), and how does a timeout deny without leaking a task/stream? (Crux of
  goal 3 — mirror the "a live process can't hang" discipline.)
- **Store durability:** is `ApprovalManager` in-memory per-replica, and does a
  multi-replica deployment need the decision to reach the replica holding the
  paused call? (Affects goals 2–3; may be a documented single-replica constraint
  this phase, like the sugar-overlay decision last phase.)
- **UI surface:** a new "Approvals" tab vs. a section on an existing page.

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] A `RequireApproval` tool call pauses, emits an approval-request event over
      AG-UI/A2UI, and resumes on allow / drops on deny — test-proven, no
      silent-allow path.
- [ ] Operators can list and decide pending approvals via the admin API + UI.
- [ ] An undecided approval times out to **deny** (bounded); expiry is purged; an
      unavailable approval channel/store fails closed — every path test-proven.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new
      features ≥80% covered; every new approval/stream/reload path fail-safe
      (tested); web build green; separated security review on each
      approval/authz/stream-touching change.
