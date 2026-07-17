# add-pending-approvals-surface

**Phase:** agent-approval-and-step-up-flows · **Goal:** G2 (build-002)
**Scope:** `crates/flint-gate-core/src/approval/mod.rs` (list method),
`crates/flint-gate-core/src/admin/mod.rs` (GET endpoints), `web/src/` (Approvals
tab: page, client fns, hooks, types, route/nav), docs.
**Depends on:** `add-approval-timeout-and-janitor` (list should skip/reflect the
TTL semantics that change lands).

## Why

The only way to resolve an approval today is a raw `POST /approvals/{id}/decision`
with an out-of-band-known id — there is **no list endpoint** (`ApprovalManager`
exposes only single-id `status()`; `len`/`is_empty` are `#[cfg(test)]`) and **no
web UI** at all. An operator cannot see what is pending, so the human-in-the-loop
flow is not actually operable.

## What

1. **`ApprovalManager::list()`** — a production method returning
   `Vec<ApprovalStatus>` (iterate the DashMap, skipping already-expired entries).
   `ApprovalStatus` is already `Serialize`.
2. **Admin GET endpoints** — `GET /approvals` (list pending) and `GET
   /approvals/{id}` (single status, reusing `status()`), on the admin router,
   mirroring the existing policy / agent-identity list handlers.
3. **Approvals web tab** — a new `/approvals` route + nav in `App.tsx`, a
   `pages/Approvals.tsx` listing pending approvals (id, principal, action/tool,
   expiry) with **approve / deny** buttons posting to `/approvals/{id}/decision`,
   plus client fns, React-Query hooks, and types. Uses a TanStack Query
   `refetchInterval` **poll** so an operator sees new / expiring approvals without
   manual reload (pending approvals are time-sensitive). Follows the
   AgentIdentities / Policies page pattern.

## Non-goals

- Cross-replica visibility — the list reflects the **local replica's** pending
  approvals only (documented single-replica constraint; see the timeout change).
- Showing decided/expired history — pending only (decided outcomes are already in
  the authz audit trail).
- Bulk approve/deny.

## Fail-safe requirement

The GET endpoints are **admin-router only** (loopback/private by default, gated by
the existing admin exposure posture) — never on the public proxy. `list()` skips
expired entries so a stale approval can't be actioned. Deciding an already-
expired/absent approval returns the existing 410/404 (unchanged). Tested: list
returns pending, skips expired; endpoints admin-only.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage on the list method + GET handlers; web build + typecheck green; the
Approvals tab lists and decides. Separated security review (admin-only exposure;
no sensitive leak in the list payload).
