# Reflection — agent-approval-and-step-up-flows

_Reflected 2026-07-08. All 3 changes archived. Workspace green: 492 tests (0
failed). Web build + typecheck green._

---

## Goal Achievement

| Goal | Description | Verdict |
|------|-------------|---------|
| G1 | End-to-end pause → request → resume/deny over the wire | **MET** |
| G2 | Pending-approvals operator surface (admin API + UI) | **MET** |
| G3 | Fail-closed lifecycle: timeout, expiry, unavailability | **MET** |

**Overall: 3/3 goals MET (100%)**

### G3 — Fail-closed lifecycle (change 1: `add-approval-timeout-and-janitor`)

- Added `approval: { enabled, ttl_seconds }` config block to `config/types.rs`;
  `enabled: false` now fails-closed to Deny at the decision/stream boundary —
  a `RequireApproval` with no approval channel configured never silently allows.
- Added a `sleep_until(nearest expires_at)` arm in the `select!` loop of
  `middleware/pipeline.rs`; on fire, the held call is resolved as Deny (deny
  event emitted + stream terminated, no half-open stream).
- Spawned a background `tokio::time::interval` janitor in `main.rs` calling
  `ApprovalManager::purge_expired()` periodically.
- Tests: timeout→deny for one and two staggered approvals; janitor reaps;
  `enabled:false` denies; existing pause/resume unregressed.
- Security review (separated): HIGH-3 (decide removes before expiry check →
  retry callers got `404 NotFound` instead of `410 Gone`) **fixed via
  peek-before-remove**. HIGH-2 (admin rate-limit) and MEDIUM-1 (unbounded
  DashMap) noted as pre-existing architectural items for future phases.

### G2 — Operator surface (change 2: `add-pending-approvals-surface`)

- `ApprovalManager::list()` added: iterates DashMap, skips expired entries.
- `GET /approvals` and `GET /approvals/{id}` added to the admin router (never
  public; mirrored from the policy/agent-identity handler pattern).
- Web: `listApprovals`/`getApproval`/`decideApproval` in `admin.ts`;
  React-Query hooks with `refetchInterval: 5_000` in `useAdmin.ts`; approval
  types in `types.ts`; `/approvals` route + nav + `pages/Approvals.tsx` table
  (approval ID, principal, action/badge, resource, reason, expires-in,
  approve/deny icon buttons with toast feedback).
- 6 new backend tests; web build + typecheck green.
- Security review (separated): WARN — HIGH-3 addressed (see G3), HIGH-2
  pre-existing, MEDIUM-1 pre-existing.

### G1 — End-to-end flow comments + verification (change 3: `fix-approval-flow-comments-and-verify`)

- Fixed misleading `approval_handle` field comments in `ag_ui.rs` and
  `a2ui.rs` — the "fail-closed deny" wording now correctly scopes to the
  no-handle fallback only; the live path is accurately described as
  pause→await-decision→resume/deny.
- Added two no-silent-allow invariant tests in `processor.rs`:
  `ag_ui_require_approval_without_handle_is_denied_not_silently_allowed` and
  `a2ui_require_approval_without_handle_is_denied_not_silently_allowed` — these
  are regression guards for the security invariant that no-handle →
  fail-closed, never silent-allow.
- Added sequential flow overview to `### Human-in-the-loop approval` in
  README.md: numbered 1–6 steps (Cedar → PAUSE → gate:approval_request →
  operator decides → approve releases / deny drops → timeout auto-deny) plus
  the no-handle fast-path paragraph.

---

## Artifact Quality Summary

| Metric | Value |
|--------|-------|
| Changes with QA | 3/3 |
| Separated security reviews | 2/3 (changes 1 + 2; change 3 was verify/docs — code-reviewer pass) |
| Security findings addressed this phase | 1 CRITICAL/HIGH (HIGH-3 peek-before-remove fix) |
| Pre-existing security items deferred | 2 (HIGH-2 admin rate-limit, MEDIUM-1 unbounded DashMap) |
| Test suite at phase close | 492 passed, 0 failed, 8 ignored |
| Test suite at phase open | 475 passed |
| New tests added this phase | ~17 net new (timeout, janitor, list/GET endpoints, no-silent-allow × 2) |
| Web build | ✓ clean (tsc -b + Vite) |
| Cargo clippy -D warnings | ✓ clean |

No artifact-refiner logs found (`.refiner/artifacts/` skips this phase's
changes). QA was carried out via separated security-reviewer agent (changes
1 & 2) and manual code-reviewer pass (change 3).

### Security findings summary

- **HIGH-3 (FIXED)**: `ApprovalManager::decide()` removed the DashMap entry
  before checking expiry — a retry caller after the first `410 Gone` would get
  `404 NotFound`. Fixed with peek-before-remove: `get()` checks expiry,
  `remove()` only runs on a live entry. Expired entries remain for the janitor.
- **HIGH-2 (deferred)**: No rate-limit on the admin `POST /approvals/{id}/decision`
  endpoint. Pre-existing pattern across all admin write endpoints. Candidate for
  a dedicated admin-hardening change.
- **MEDIUM-1 (deferred)**: `ApprovalManager` uses an unbounded `DashMap`. A
  burst of `RequireApproval` decisions (e.g., under a misconfigured policy)
  could grow unbounded before the janitor runs. Bounded with a max-pending
  cap is a future hardening item.

---

## Technical Debt Introduced

- **Single-replica ApprovalManager constraint** is documented but not enforced.
  A multi-replica deployment will silently split approvals across replicas (the
  decision may reach the wrong replica). A future phase should add either
  sticky routing or a shared store (Redis pub/sub). This was an explicit
  scope decision this phase.
- **Janitor interval is hardcoded** to 60s in `main.rs`. Should join the
  `approval` config block as `janitor_interval_seconds` in a follow-on change.
- **No admin rate-limit** on decision endpoints (HIGH-2, deferred above).
- **In-band decision channel** (streaming client posts decision over the same
  connection) is out of scope. The current operator-REST + admin-UI path is
  complete for the single-operator case; multi-operator quorum would need it.

---

## Lessons Captured

1. **Assess before spec — always.** The seed put G1 (end-to-end flow) first as
   "not built." Assess proved it was already implemented end-to-end. The order
   inverted: G3 (the actual safety hole — hang-forever) → G2 (surface) → G1
   (verify + polish). Without the assess step the team would have wasted two
   changes re-implementing working code.

2. **Peek-before-remove is the correct DashMap decide pattern.** Concurrent
   `decide()` calls on an expired entry must consistently see `Expired` (410),
   not `NotFound` (404) after the first remove. The fix is `get()` for the
   expiry check, then `remove()` only on a confirmed-live entry. The two-concurrent-
   racer case (second `remove()` returns `None`) falls through to `NotFound` —
   acceptable and documented.

3. **Separated security review on every approval/stream/authz seam.** The
   author-never-grades-own-work rule paid off: HIGH-3 was found by the separated
   reviewer, not by the author. Wire this into the plan constraint for any
   future change touching `ApprovalManager`, `middleware/pipeline.rs`, or
   admin decision endpoints.

4. **Fail-closed at every edge means no silent-allow path at any level.** Three
   edges secured this phase: (a) `enabled:false` → immediate deny at decision
   boundary; (b) `None` approval handle → deny in processor (regression-tested);
   (c) TTL expiry → auto-deny via `sleep_until` in the `select!`. A future
   reviewer adding a new edge must prove it also closes, not opens.

5. **Comments that describe only one branch are a reader trap.** The original
   `approval_handle` comment said "RequireApproval is treated as a deny
   (fail-closed)" without qualifying "only when handle is absent." A future
   maintainer reading that comment would likely "fix" the flow into an always-deny.
   Misleading comments in security-critical paths deserve the same fix priority
   as bugs.

6. **Single-replica memory state needs a scope decision at the start of spec.**
   The `ApprovalManager` in-memory constraint was surfaced at assess time and
   explicitly scoped out — the right call for a focused phase. Capturing it as
   a deferred debt item at spec time (not at the end of the phase) would have
   been cleaner; add it to the assess checklist for any future in-memory shared
   state.

---

## Recommended Next Phase

### Option A — Admin server hardening + multi-replica approval routing

**Rationale:** The two deferred items from this phase (HIGH-2 admin rate-limit,
MEDIUM-1 unbounded DashMap) plus the single-replica constraint are the sharpest
remaining safety gaps for a real deployment. Hardening the admin server (rate-limit
on all write endpoints, request-size cap, explicit CORS policy) and documenting
(or implementing) a routing strategy for multi-replica approval decisions would
make the gateway production-ready at the interactive governance layer.

**Scope:**
- Admin write-endpoint rate-limiting (HIGH-2)
- `ApprovalManager` max-pending cap + janitor config (MEDIUM-1 + hardcoded 60s)
- Multi-replica approval routing: sticky sessions or Redis pub/sub
  (scope: at least document the deployment constraint + add integration test)
- Admin auth strengthening audit (token rotation, session expiry)

### Option B — Quorum / multi-approver approval policies

**Rationale:** The current flow supports single-decision approval. A Cedar policy
today can only express `@require_approval`; it cannot say "require 2-of-3
approvers." Quorum approval is a natural extension that makes the gateway useful
for higher-stakes tool calls (e.g., destructive database mutations, production
deployments).

**Scope:**
- `ApprovalManager` quorum tracking (N approvers, M required)
- Cedar annotation extension: `@require_approval("reason", quorum=2)`
- Admin UI: pending-approval list shows quorum status (1/2, 2/2)
- Test: partial approval pauses; full quorum resumes; any deny drops

### Option C — LLM-ops bundle (semantic caching, multi-LLM routing)

**Rationale:** Deferred across two phases as out-of-scope. If the team's next
priority shifts from governance to performance/cost, this is the queued work.

**Recommendation:** **Option A first** — the safety gaps from this phase are
known, scoped, and directly blocked on by any production deployment of the
approval flow. Option B is valuable but additive; Option A is correctness.
