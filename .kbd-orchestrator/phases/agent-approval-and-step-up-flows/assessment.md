# Assessment — agent-approval-and-step-up-flows

_Assessed 2026-07-08 against `goals.md`, via a thorough two-track trace of the
`RequireApproval` flow (engine → stream processors → ApprovalManager → decision
endpoint) and the operator/lifecycle surfaces. Workspace green at entry (475 core
tests; prior two phases committed + pushed)._

## Headline: the phase is smaller and sharper than seeded

The seed hypothesized the human-in-the-loop flow was "not built out." **That is
wrong** — the pause→request→resume/deny flow is **already implemented end-to-end
and genuinely pauses** (not a fail-closed deny; the "treated as a deny" comments
annotate only the no-handle fallback). The real work is narrower and safety-shaped:

- **G1 (end-to-end flow): MOSTLY MET** — verify/polish + fix stale comments; one
  optional gap (in-band client decision channel).
- **G2 (operator surface): NOT MET** — no list endpoint, no `ApprovalManager`
  iterate method, no web UI.
- **G3 (fail-closed lifecycle): NOT MET — the genuine safety gap** — an undecided
  approval **hangs the paused stream forever** (no timeout) and leaks the pending
  entry (`purge_expired` is never called). No approval config exists.

**G3 is now the highest-priority item** (it's a fail-open-to-hang safety hole),
inverting the seeded build order (which put the flow first).

---

## G1 — End-to-end pause → request → resume/deny  ·  Verdict: MOSTLY MET

**What exists (all wired):**
- Engine: `AuthzDecision::RequireApproval(ApprovalContext{approval_id, principal_id,
  action, resource_id, reason, expires_at})` (`authz/engine.rs:62-76,132`), produced
  from the `@require_approval` annotation on a contributing permit
  (`extract_approval_context`, `engine.rs:414-466`); fail-closed to Deny on any
  error. `DEFAULT_APPROVAL_TTL_SECONDS = 300` (`engine.rs:54`).
- `ApprovalManager` (`approval/mod.rs`): `register`/`decide`/`status`/`purge_expired`,
  DashMap-backed, correlates a pending approval with the paused stream via an
  `UnboundedSender<(String, ApprovalDecision)>`; per-approval `expires_at`.
- **Genuine pause + resume:** AG-UI (`stream/ag_ui.rs:500,679` → `request_approval`
  buffers held START/ARGS/END, emits `GATE_APPROVAL_REQUEST`) and A2UI
  (`stream/a2ui.rs:224`, request event emitted by `processor.rs:399`). The pipeline
  loop reads upstream **only while `pending_approvals().is_empty()`**, else awaits a
  decision and `resolve_approval` flushes (allow) or drops (deny)
  (`middleware/pipeline.rs:783-833`). Works for SSE + NDJSON.
- Decision ingress: `POST /approvals/{id}/decision` (`admin/mod.rs:170,1159`) →
  `decide` → resumes/denies. Event types are in the AG-UI/A2UI schemas.

**Gaps (small):**
- **Stale/misleading comments** at `ag_ui.rs:245` + `a2ui.rs:140` ("RequireApproval
  is treated as a deny (fail-closed)") describe only the `None`-handle branch; the
  live path pauses. Fix the comments so a future reader doesn't "correct" the flow
  into a deny.
- **Decision ingress is Admin-REST-only** — no in-band path for the streaming
  client to post its decision back over the same connection. Whether this is a gap
  depends on scope; the emitted request event + out-of-band REST decision is a
  complete (if less convenient) flow. **Analyze/Spec decides** if an in-band
  decision channel is in-scope this phase (lean: OUT — REST + UI covers the
  operator case; in-band client-decides is a larger stream-protocol change).

**Effort:** LOW (comment fix + a verification test that the flow pauses/resumes;
the in-band channel, if pursued, is MEDIUM and separable).

---

## G2 — Pending-approvals operator surface  ·  Verdict: NOT MET

**What's missing:**
- **No list/status endpoint.** The only approval route is
  `POST /approvals/{id}/decision`. No `GET /approvals` (list) or `GET /approvals/{id}`
  (status) (`admin/mod.rs:170`).
- **No iterate method on `ApprovalManager`.** Only single-id `status(id)`
  (`approval/mod.rs:145`); `len`/`is_empty` are `#[cfg(test)]`. **A list endpoint
  cannot be built without adding a production `list()`/iterate method** that returns
  `Vec<ApprovalStatus>` (the type is already `Serialize`).
- **No web UI.** No `pages/Approvals.tsx`, no `/approvals` route/nav in `App.tsx`,
  no approval client fn/hook in `admin.ts`/`useAdmin.ts`, no approval type in
  `types.ts` (only the `AuthzDecision` audit label). Today the only way to resolve
  an approval is a raw REST POST with an out-of-band-known id — unusable as an
  operator surface.

**Design note (reachability of the decision):** `ApprovalManager` is **in-memory
per-replica** (DashMap, not shared). The decision must reach the replica holding the
paused stream. A list endpoint on replica A won't show replica B's pending
approvals, and a decision POST to A can't resolve B's stream. This is the crux
open question for G2 (see below) — likely a documented single-replica constraint
this phase (mirrors the sugar-overlay decision), with sticky-routing / shared store
as a follow-up.

**Effort:** MEDIUM (add `list()` iterate + `GET /approvals` + `GET /approvals/{id}`;
web Approvals page following the AgentIdentities/Policies pattern already used).

---

## G3 — Fail-closed lifecycle (timeout / expiry / unavailability)  ·  Verdict: NOT MET — the genuine safety gap

**What's broken (fail-open-to-hang):**
- **No timeout on the paused stream.** The pipeline awaits `approval_rx.recv()`
  (`pipeline.rs:821`) with only a `stream_cancel.cancelled()` companion arm (the
  session watchdog, unrelated to approval TTL). There is **no
  `tokio::time::timeout` / deadline keyed on the approval `expires_at`**. An
  undecided approval → `recv()` blocks indefinitely → **the paused stream never
  resumes and never fails closed** (hangs until the client disconnects or the
  watchdog, if enabled, fires). This is a real availability + governance hole: the
  TTL exists on the context but is enforced ONLY inside `decide()` (i.e. only if a
  decision eventually arrives).
- **`purge_expired` is never called.** Defined (`approval/mod.rs:158`) with a
  doc saying "run periodically from a background janitor," but **no call site
  exists** (grep: only the def + tests). Expired entries leak from the DashMap
  forever (slow memory growth + stale `status` reads).
- **No approval config** (`config/types.rs` has no `approval` block): the 300s TTL
  is a hardcoded constant, not overridable; no enable/disable.
- **Partial fail-closed on channel loss:** a dropped sender → `None => break` tears
  the stream down (no unauthorized call proceeds), but it's a bare `break` with no
  synthetic deny event/`term_payload` (contrast the cancel arm). Minor; the primary
  risk is the hang, not this path.

**This is the fail-safe close the phase most needs.** The fix: add a
TTL-deadline arm to the paused-stream select (undecided → **auto-deny** the held
call, emit the deny event, resume the stream to termination) + a background
`purge_expired` janitor + a config TTL/enable knob.

**Effort:** MEDIUM (the timeout arm is the critical, security-sensitive change —
must deny the held call, not silently drop; the janitor + config are small).

---

## Cross-cutting observations

- **Zero new dependencies expected** — `tokio::time::timeout`, DashMap iterate, an
  axum GET handler, and the existing React admin kit cover all three goals.
- **Build order should INVERT the seed:** **G3 first** (close the hang — it's a live
  fail-open-to-hang defect), then **G2** (operator surface), then **G1** (comment
  fix + verification; in-band channel deferred/optional). The seed put G1 first;
  Assess corrects that — G1 is already working, G3 is the bug.
- **Fail-safe discipline carries:** G3's timeout must **auto-deny** (fail-closed),
  not drop-silently; each change touches the approval/stream/authz seam → separated
  security review each. Multi-replica reachability is the recurring "documented
  constraint vs enforced" watch-item (last two phases' pattern).
- **Blocking constraints unaffected:** no secrets; admin 4457 stays loopback
  (new GET endpoints are admin-router only); no test breakage expected; config
  priority (CLI>env>YAML) untouched (an added `approval` block is additive).

## Open questions for Analyze / Spec

1. **G3 timeout semantics** — on TTL expiry of a paused approval, auto-**deny** the
   held tool call (emit the deny event + resume stream to termination), correct?
   Where does the deadline live — a `tokio::time::timeout` on `approval_rx.recv()`
   keyed on the nearest pending `expires_at`, or a per-approval timer? (Crux of G3.)
2. **G3 config** — add `approval: { enabled?, ttl_seconds? }` to `config/types.rs`
   (overriding the 300s default), or keep the constant and only add the janitor?
3. **G2 multi-replica** — is the in-memory per-replica `ApprovalManager` a
   documented single-replica constraint this phase (list/decide only see the local
   replica), with sticky-routing / shared store a follow-up? Or in-scope now?
4. **G1 in-band decision channel** — deferred/out-of-scope (REST + UI suffices), or
   a phase goal? (Larger stream-protocol change if in.)
5. **G2 UI surface** — a new "Approvals" tab vs. a section on an existing page; and
   does the list auto-refresh (poll) so an operator sees new pending approvals?
