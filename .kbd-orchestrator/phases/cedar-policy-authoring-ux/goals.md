# Goals — cedar-policy-authoring-ux

_Seeded from `admin-hardening-and-multi-replica-approval/reflection.md` →
"Recommended Next Phase → Option A" (auto-seeded). The prior phase hardened
the admin server and made the approval flow production-deployable (rate-limit,
cap, multi-replica documentation). The Cedar policy engine is live with
hot-reload and write-time validation. What is missing is the operator-facing
experience for authoring, testing, and observing Cedar policies in practice._

## Phase Goal

**Make Cedar policies operator-usable** by delivering: a structured authoring
workflow (beyond raw file editing), an inline parse-error surface in the admin
UI, a policy test harness (simulate agent requests against policies without
live traffic), and hardened hot-reload error recovery (visible error state when
a reload produces an invalid policy rather than silent fallback).

Still **authorization-first**; still **federate any JWKS-capable IdM (Ory
reference), never an IdP**; quorum/multi-approver approval and step-up auth
stay out of scope for this phase.

**Seeded from:** `admin-hardening-and-multi-replica-approval` reflection ·
**Criteria profile:** operator-experience (unblock Cedar policy engine use in practice)

## Known starting point (VERIFY + refine in Assess)

From the prior phase reflection:

- **Cedar engine**: live with hot-reload and write-time validation; policy files
  are validated on write via the Cedar SDK.
- **Hot-reload gap**: if a policy file becomes invalid between reloads, the
  engine falls back to the last valid policy silently — no visible error state
  for the operator.
- **Admin UI policy editor**: present as a stub; does not surface Cedar parse
  errors inline; no test/simulate capability.
- **No policy test harness**: operators cannot simulate agent requests against
  a policy without sending live traffic through the gateway.
- **No authoring workflow**: operators edit raw `.cedar` files; no schema
  validation UI, no guided authoring, no diff-on-reload.

## Goals (build order — dependency-aware; refined by Assess/Spec)

1. **Hot-reload error recovery** *(correctness gap).* When a policy reload
   produces a Cedar parse/validation error, the engine must: (a) retain the
   previous valid policy, (b) emit a structured error event visible to the
   operator (log + admin API endpoint or SSE event), (c) surface the error in
   the admin UI rather than silently staying on the old policy. Add a startup
   check that the initial policy load succeeded; fail-closed (refuse to start)
   if the initial policy is invalid.
   (HIGH — current silent fallback can mask a broken policy for an entire
   restart cycle.)

2. **Admin API: policy parse/validate endpoint** *(authoring prerequisite).*
   Add `POST /admin/policies/validate` (body: raw Cedar policy text) that
   returns a structured JSON response with `valid: bool` and `errors: [...]`.
   No side effects — this is a pure validation check, not a write. Rate-limited
   by the existing admin rate-limit layer.
   (HIGH — unblocks both the UI editor and any CLI authoring workflow.)

3. **Admin UI: inline Cedar parse error surface** *(operator UX).* The policy
   editor in the admin UI should: call `POST /admin/policies/validate` on
   change (debounced), render parse errors inline (line/column if available
   from the Cedar SDK), and disable the Save button when the policy is invalid.
   (MEDIUM — depends on G2.)

4. **Policy test harness: `POST /admin/policies/simulate`** *(authoring
   workflow).* Accept a Cedar request context (principal, action, resource,
   context map) and return the authorization decision the current policy would
   produce (Allow / Deny + which policy rule matched). No side effects, no live
   traffic. Useful for operators to verify policy intent before deploying.
   (MEDIUM — the most requested feature for Cedar operability.)

## Explicitly out of scope (this phase)

- Quorum / multi-approver approval policies.
- Step-up authentication flows.
- In-band streaming decision channel.
- LLM-ops bundle (semantic caching, multi-LLM routing, prompt compression).
- Full OAuth2 authorization server / IdP.
- SAML / SCIM / LDAP federation.
- Policy versioning / rollback history (future phase).

## Open questions (resolve during Assess/Analyze)

- **Cedar SDK error format:** does the Rust Cedar SDK return structured
  line/column error information, or only a message string? This determines
  how rich the validate endpoint response can be.
- **SSE vs. polling:** should the hot-reload error state be pushed via SSE
  (the admin UI already has an SSE event stream) or polled via a
  `GET /admin/policies/status` endpoint?
- **Simulate scope:** should `/simulate` evaluate against the current live
  policy only, or allow the caller to supply an arbitrary policy text to
  simulate against? The latter is more powerful but requires careful sandboxing.
- **Admin UI framework:** is the admin UI (stub) in a JS/TS framework (React,
  Vue, etc.) or is it server-rendered? This affects the inline error surface
  implementation complexity.

## Success criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] Hot-reload with an invalid policy emits a structured error and retains
      the previous valid policy; the admin API surfaces the error state.
- [ ] Gateway refuses to start if the initial policy load fails (fail-closed).
- [ ] `POST /admin/policies/validate` returns `{ valid, errors }` with correct
      Cedar parse results; covered by unit + integration tests.
- [ ] Admin UI policy editor shows inline parse errors and disables Save on
      invalid input.
- [ ] `POST /admin/policies/simulate` returns Allow/Deny + matched rule for a
      given Cedar request context.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`;
      new features ≥80% covered; web build green where applicable.
