# Goals — admin-hardening-and-multi-replica-approval

_Seeded from `agent-approval-and-step-up-flows/reflection.md` →
"Recommended Next Phase → Option A" (operator-selected). The prior phase
made human-in-the-loop approval a complete, observable flow and closed the
fail-closed lifecycle gap. Two security items were deferred (HIGH-2 admin
rate-limit, MEDIUM-1 unbounded DashMap) and the in-memory single-replica
constraint was explicitly scoped out. This phase closes those gaps and makes
the approval flow production-deployable._

## Phase Goal

**Harden the admin server** and **resolve the multi-replica approval routing
problem** so the gateway's interactive governance layer is safe to operate
at production scale: rate-limit all admin write endpoints, cap the
`ApprovalManager` at a bounded size, expose the janitor interval as config,
and either implement or conclusively document (with a test) the sticky-routing
or shared-store strategy for multi-replica deployments.

Still **authorization-first**; still **federate any JWKS-capable IdM (Ory
reference), never an IdP**; the LLM-ops bundle and quorum/multi-approver
approval stay out of scope.

**Seeded from:** `agent-approval-and-step-up-flows` reflection ·
**Criteria profile:** effort-impact (safety gaps first)

## Known starting point (VERIFY + refine in Assess)

From the prior phase reflection:

- **HIGH-2 (deferred):** No rate-limit on `POST /approvals/{id}/decision` or
  any other admin write endpoint. Pre-existing pattern across all admin write
  handlers. Tower middleware or per-handler governor integration is the likely
  fix.
- **MEDIUM-1 (deferred):** `ApprovalManager` inner `DashMap` is unbounded. A
  burst of `RequireApproval` decisions under a misconfigured policy grows
  unbounded before the janitor runs.
- **Janitor interval hardcoded** at 60s in `main.rs` — should be
  `approval.janitor_interval_seconds` in the config block.
- **Single-replica constraint:** `ApprovalManager` is in-memory per-replica.
  A `POST /approvals/{id}/decision` reaching the wrong replica gets `404
  NotFound` (the entry lives on the replica holding the paused stream). No
  routing solution exists yet.
- **Admin auth audit:** token rotation and session expiry on the admin server
  have not been reviewed this phase-line; surface any gaps during Assess.

## Goals (build order — dependency-aware; refined by Assess/Spec)

1. **Admin write-endpoint rate-limiting** *(HIGH-2 closure).* Apply a
   configurable rate-limit to all admin write routes (`POST /approvals/{id}/decision`,
   `POST /policies`, `POST /agent-identities`, `DELETE /*`, etc.) via Tower
   middleware or per-handler governor. Return `429 Too Many Requests` on
   breach. Configurable: `admin.rate_limit.requests_per_second` + burst.
   (HIGH — security gap.)

2. **`ApprovalManager` max-pending cap + bounded janitor config** *(MEDIUM-1
   closure + config debt).* Add `approval.max_pending: Option<usize>` (default
   sensible, e.g. 1000); when at cap, new `register()` calls fail-closed
   (return `ApprovalError::CapExceeded` → Deny at the stream boundary, never
   block). Expose `janitor_interval_seconds` in the config block (replace the
   hardcoded 60s in `main.rs`). (MEDIUM.)

3. **Multi-replica approval routing: document + integration-test the constraint**
   *(single-replica gap).* Assess decides the implementation strategy:
   - **Option 3a (sticky routing):** add an `X-Approval-Replica-ID` header or
     sticky session hint so the load balancer routes `POST /approvals/{id}/decision`
     to the replica that registered the approval.
   - **Option 3b (shared store):** replace the inner `DashMap` with a Redis
     pub/sub or similar shared backing store so any replica can resolve any
     pending approval.
   - **Option 3c (documented constraint + integration test):** if 3a/3b are
     out of scope for this phase, write an integration test that FAILS when a
     decision reaches the wrong replica, document the constraint prominently
     (config comment + README), and emit a startup warning when `REPLICA_COUNT
     > 1` is detected. This is the minimum acceptable for shipping.
   
   Assess/Analyze decides which option fits. (MEDIUM — correctness gap for
   multi-replica deployments.)

4. **Admin auth audit** *(gap sweep).* Review the admin server auth path
   (JWT validation, token rotation, session expiry, CORS) for any gaps not
   addressed in prior phases. Produce a finding list; address CRITICAL/HIGH
   findings as tasks in this change; defer MEDIUM/LOW with debt notes.
   (LOW-MEDIUM — sweep, not a known hole.)

## Explicitly out of scope (this phase)

- Quorum / multi-approver approval policies (single-decision HITL this phase-line).
- In-band streaming decision channel (operator-REST + UI is the complete path).
- LLM-ops bundle (semantic caching, multi-LLM routing, prompt compression).
- Full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.

## Carried-over open questions (resolve during Assess/Analyze)

- **Multi-replica option:** which of 3a/3b/3c fits the deployment model and
  team capacity? Assess should check if a load balancer or service mesh is
  already in the stack.
- **Rate-limit library:** `tower-governor` vs. `axum-ratelimit` vs. custom
  Tower layer? Analyze should score options against maintenance health and
  axum compatibility.
- **Cap on `max_pending`:** what is a sensible default that protects memory
  without blocking legitimate bursts? Check the approval TTL and expected
  agent concurrency.
- **Admin CORS:** is the admin server exposed via a reverse proxy that handles
  CORS, or does it need explicit CORS middleware?

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] All admin write endpoints return `429` when the configured rate is
      exceeded; rate-limit config (`requests_per_second`, `burst`) is live in
      `config.example.yaml`.
- [ ] `ApprovalManager::register()` returns `CapExceeded` (→ Deny) when
      `max_pending` is reached; `janitor_interval_seconds` is configurable.
- [ ] Multi-replica constraint is either resolved (3a or 3b) or explicitly
      tested to fail and documented with a startup warning (3c).
- [ ] Admin auth audit complete; CRITICAL/HIGH findings addressed or none found.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`;
      new features ≥80% covered; web build green where applicable.
