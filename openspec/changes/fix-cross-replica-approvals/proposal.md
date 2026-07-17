# fix-cross-replica-approvals

**Phase:** beta-release-readiness / Phase 1 (Blocker B-1)
**Priority:** BLOCKER — must close before any external beta

## Problem

`ApprovalManager` is in-process, per-replica. The K8s manifest has
`replicas: 2`. A `POST /approvals/:id/decision` request that lands on the
wrong replica returns `NotFound` — silently, with no retry guidance. In a
2-replica deployment, roughly 50% of approval decisions fail.

The config comment says "cross-replica routing is a follow-up" but the shipped
k8s manifest already has 2 replicas. This is a live regression, not a future
gap.

## Solution (beta-scoped: sticky sessions)

For beta, use `sessionAffinity: ClientIP` on the admin service to ensure
approval decisions from the same admin client reach the same replica. This
is adequate for a small beta cohort where one admin makes all decisions from
one client.

A Postgres-backed shared approval store is the correct long-term fix but is
out of scope for this phase — it requires a new DB table and transaction
semantics.

## Files to change

- `k8s/service.yaml` (or new `k8s/service-admin.yaml`) — add `sessionAffinity: ClientIP`
- `crates/flint-gate/src/main.rs` — improve the multi-replica warning to be specific about the sticky session requirement
- `config.example.yaml` — document the per-replica constraint in the `approval:` block
- `docs/docs/getting-started.md` — add multi-replica deployment checklist section

## Known limitation (document, not fix)

`sessionAffinity: ClientIP` is a best-effort sticky mechanism. If a pod is
restarted, existing sessions lose their affinity and a pending stream may
lose its approval binding. Document this explicitly as a known beta limitation.
