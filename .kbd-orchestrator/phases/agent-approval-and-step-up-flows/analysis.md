# Analysis — agent-approval-and-step-up-flows

_Analyzed 2026-07-08. Mode: build-vs-adopt for three approval-flow gaps. Assess
established the flow is already built (G1 mostly met) and the real work is G3
(fail-closed lifecycle — the safety gap) + G2 (operator surface), with **zero
external adoption surface** — every gap is closed with in-tree `tokio::time`, a
DashMap iterate, an axum GET handler, and the existing React admin kit. So this
Analyze is **design-decision resolution + one in-tree pattern confirmation**, not
library discovery._

## Research pipeline outcome

- **Tier 1 (gh search):** N/A — no framework/skeleton to adopt; the approval
  engine, `ApprovalManager`, stream pause/resume, and admin CRUD patterns already
  exist in-repo. A HITL-approval crate would duplicate working code.
- **Tier 2 (docs):** the one thing worth confirming — the `tokio::time` deadline
  idiom for the G3 auto-deny — is confirmed **in-tree, not by external docs**:
  `tokio = { features = ["full"] }` (so `time` is present) and the codebase already
  uses `tokio::time::interval` (`pipeline.rs:744`, the session watchdog in the SAME
  file), `sleep` (`cache/mod.rs:386`), and `timeout` (`main.rs:839`). The paused-
  stream `select!` at `pipeline.rs:815-833` already has the exact shape a deadline
  arm slots into. Docfork was unreachable; the idiom is standard + in-tree, so no
  Tier-4 spend was warranted.
- **Tier 3 (registries):** N/A — no new crate/npm candidate.
- **Tier 4 (broad web):** not run (Tiers 1–3 sufficient; the one confirmation was
  in-tree).

**Build-vs-adopt verdict: BUILD (wire), all three goals. ZERO new dependencies.**
No `adopt`/`adapt` candidates — see `library-candidates.json` (`build_required[]`
only).

## Design decisions (open questions → resolved)

### D1 (G3) — Paused-stream timeout → auto-DENY (the safety-critical fix)
**Decision:** add a **third arm** to the paused-stream `select!`
(`pipeline.rs:815-833`) — `_ = tokio::time::sleep_until(deadline) => …` — where
`deadline` is the **nearest pending approval's `expires_at`** (converted to a tokio
`Instant`). On fire, the held tool call is resolved as **Deny** (emit the deny
event via `resolve_approval` with a synthetic Deny, resume the stream to
termination), NOT silently dropped. · **Provenance:** Assess (`recv()` at
`pipeline.rs:821` has no deadline → hangs forever) + in-tree tokio patterns
(watchdog `interval` at `pipeline.rs:744`, cancel arm at `:817`). · **Rationale:**
mirrors the existing cancel arm's structure; auto-deny is the fail-closed analog
for an undecided approval (never silent-allow, never hang). The deadline is the
nearest `expires_at` so a single sleep covers the frontmost pending approval; on
resolve, recompute for the next. `ApprovalManager` already stores `expires_at:
Instant` (monotonic) — reuse it, don't re-derive from wall clock.

### D2 (G3) — `purge_expired` janitor
**Decision:** spawn a **background interval task** (mirror the watchdog
`tokio::time::interval` at `pipeline.rs:744`) that calls
`ApprovalManager::purge_expired()` periodically (e.g. every TTL/2 or a fixed 60s),
started in `main.rs` next to the other background tasks. · **Provenance:** Assess
(`purge_expired` defined at `approval/mod.rs:158`, never called; its own doc says
"run from a background janitor"). · **Rationale:** closes the DashMap leak; the
interval pattern is already used in-repo. Note: the D1 stream-timeout is the
*correctness* fix (the stream no longer hangs); the janitor is *hygiene* (reap
entries whose streams already ended) — both are needed, they cover different
lifecycles.

### D3 (G3) — Approval config block
**Decision:** add an `approval: { enabled: bool (default true), ttl_seconds:
Option<u64> }` block to `config/types.rs` (serde default), overriding the hardcoded
`DEFAULT_APPROVAL_TTL` (300s). `enabled: false` makes a `RequireApproval` decision
**fail closed to Deny** (an operator who can't service approvals denies rather than
hangs). · **Provenance:** Assess (no `approval` in config; 300s is a constant). ·
**Rationale:** consistent with the phase-line's config-driven fail-safe posture;
`enabled` gives an operator a kill-switch that fails closed. Keep it minimal (TTL +
enable) — per-route approval config is a follow-up.

### D4 (G2) — List endpoint + `ApprovalManager` iterate
**Decision:** add `ApprovalManager::list() -> Vec<ApprovalStatus>` (iterate the
DashMap, skip expired) + `GET /approvals` (list) and `GET /approvals/{id}` (reuse
`status`) on the admin router. `ApprovalStatus` is already `Serialize`. ·
**Provenance:** Assess (only single-id `status`; `len`/`is_empty` are
`#[cfg(test)]`; no GET route). · **Rationale:** minimal backing method unblocks the
operator surface; GET endpoints mirror the existing policy/agent-identity list
handlers.

### D5 (G2) — Web UI surface
**Decision:** a **new "Approvals" tab** (route + nav in `App.tsx`, `pages/Approvals.tsx`,
client fns + hooks + types) following the AgentIdentities/Policies page pattern,
with a **poll refresh** (TanStack Query `refetchInterval`) so an operator sees new
pending approvals without manual reload — approve/deny buttons post to
`/approvals/{id}/decision`. · **Provenance:** Assess (no UI at all) + the existing
React/TanStack-Query kit. · **Rationale:** pending approvals are time-sensitive
(they expire), so a poll is justified here (unlike the mostly-static Policies list);
a new tab matches the "one surface per governance concern" IA.

### D6 (G2/G3) — Multi-replica reachability
**Decision:** **document a single-replica constraint this phase** — `ApprovalManager`
is in-memory per-replica, so a list/decision only sees/resolves the local replica's
pending approvals. A shared store (Redis) + sticky routing so a decision reaches the
replica holding the paused stream is a **follow-up**. · **Provenance:** Assess
(DashMap, per-replica). · **Rationale:** matches the last two phases' "documented
constraint, not silently broken" discipline (sugar-overlay, budget windows);
building cross-replica approval routing is a materially larger effort out of scope
for closing the hang + surfacing the operator view. Must be **loudly documented**
(README + config) so an operator running multiple replicas isn't surprised.

### D7 (G1) — In-band client decision channel
**Decision:** **OUT of scope this phase** — the emitted `GATE_APPROVAL_REQUEST` /
`gate:approval_request` event + the Admin REST decision endpoint + the new UI is a
complete operator flow. An in-band path for the streaming client to post its own
decision over the same connection is a larger stream-protocol change and a separate
concern (client-decides vs operator-decides). · **Provenance:** Assess. ·
**Rationale:** keeps G1 to comment-fix + verification; the in-band channel is a
clean follow-up if a use case demands client-side approval.

## Risks / watch-items

- **D1 auto-deny must emit the deny event, not just drop** — the highest-risk
  detail: on timeout the held call must be denied *and the stream resumed to a
  clean termination* (mirror the Deny path in `resolve_approval`), else the client
  hangs on a half-open stream. Separated security review must confirm no
  silent-allow and no half-open leak.
- **D1 deadline recomputation** — after resolving one approval, the next pending
  one's `expires_at` becomes the new deadline; an off-by-one (using a stale
  deadline) could over- or under-wait. Test with two staggered pending approvals.
- **D3 `enabled:false` semantics** — must fail closed to Deny at the decision point
  (a `RequireApproval` with approvals disabled = Deny), not allow. Test.
- **D6 multi-replica** — the documented constraint is a real limitation; the janitor
  + timeout are per-replica (fine — each replica reaps/denies its own).

## Open questions carried to Spec

- D1: exact deadline source — nearest `expires_at` across all pending (recomputed
  on each resolve), or a per-approval timer map? (Spec picks; nearest-deadline is
  simpler and sufficient for the common single-pending case.)
- D2 janitor interval — fixed 60s vs TTL-derived? (Spec/config decision.)
- D5: does the Approvals tab also show recently-decided (audit) or only pending?
  (Lean: pending only; decided history is in the authz audit trail already.)
