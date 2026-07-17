# Plan — agent-approval-and-step-up-flows

_Planned 2026-07-08. Backend: openspec · Driver: /kbd-apply (one task/turn).
3 changes, dependency-ordered. Zero new dependencies (from
`library-candidates.json` — all `build_required`, no adopt/adapt)._

## Ordered change list

| Order | Change | Goal | Depends on | Tasks | Recommended reviewer |
|-------|--------|------|-----------|-------|----------------------|
| 1 | `add-approval-timeout-and-janitor` | G3 | — (build first, safety-critical) | 6 | security-reviewer (**auto-deny: no silent-allow / no half-open stream — highest priority**) |
| 2 | `add-pending-approvals-surface` | G2 | **change 1** | 6 | security-reviewer (admin-only exposure; no list-payload leak) |
| 3 | `fix-approval-flow-comments-and-verify` | G1 | — (independent, last) | 3 | code-reviewer (verify-only; no behavior change) |

## Ordering rationale (INVERTS the seeded order)

The seed put the end-to-end flow (G1) first, assuming it was unbuilt. Assess
proved the flow already works, so the order inverts:

- **G3 first** — the fail-open-to-hang defect (an undecided approval hangs the
  paused stream forever; `purge_expired` never runs). This is the sharpest safety
  gap and independent, so it lands first to close the live hole.
- **G2 second** — the operator surface (list endpoint + UI). **Hard-ordered after
  G3** so the list reflects the TTL/expiry semantics change 1 establishes (skip
  expired) rather than surfacing entries the janitor is about to reap.
- **G1 last** — comment fix + an end-to-end verification test. The flow already
  works; this locks it and corrects the misleading comments. Independent, smallest,
  so it rides last (and its verification test complements change 1's timeout tests).

## Per-change reuse annotations (from library-candidates.json → build_required)

- **`add-approval-timeout-and-janitor`** reuses: `middleware/pipeline.rs:815-833`
  (the paused-stream `select!` — add the `sleep_until` arm) + `pipeline.rs:744`
  (the watchdog `tokio::time::interval` pattern for the janitor); `approval/mod.rs`
  (`expires_at` Instant, `purge_expired`); `config/types.rs` (add `approval`
  block); `tokio::time` already in-tree (features=full).
- **`add-pending-approvals-surface`** reuses: `approval/mod.rs` (`status`,
  `ApprovalStatus` Serialize — add `list()`); `admin/mod.rs` list-handler pattern
  (`list_policies_handler` / `list_agent_identities_handler`) + the existing
  `POST /approvals/{id}/decision`; `web/src/pages/{AgentIdentities,Policies}.tsx`
  + `App.tsx` + `admin.ts`/`useAdmin.ts`/`types.ts`.
- **`fix-approval-flow-comments-and-verify`** reuses: `stream/ag_ui.rs` +
  `stream/a2ui.rs` (`request_approval`/`resolve_approval`); the pipeline pause loop
  for the end-to-end test.

## Per-change QA gate (uniform)

Each change, on reaching its last task:
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<id>/refinement_log.md`.
2. **separated security-reviewer** on changes 1 & 2 (approval/stream/authz seam —
   author never grades its own); **change 1 requires an explicit "no silent-allow,
   no half-open stream on timeout" check** (highest-priority security item this
   phase). Change 3 is verify-only (documentation + test) — a code-reviewer pass
   suffices.
3. A **fail-closed / fail-safe test** on every new path (change 1: timeout→deny +
   enabled:false→deny; change 2: list skips expired + admin-only; change 3:
   no-silent-allow assertion).
4. `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` green;
   ≥80% new-code coverage; web build + typecheck green (change 2).
5. Add the `specs/` delta (already written per change) so `verify` passes on merit;
   archive via `openspec archive <id> --skip-specs --yes`; sync `progress.json` +
   advance waypoint in SEPARATE commands (pipeline-enforce hook).

## Constraints carried (from .kbd-orchestrator/constraints.md)

- No secrets / signing keys / prod DB creds committed.
- Admin server (4457) never public — the new `GET /approvals` endpoints are
  admin-router only.
- No broken existing tests; config priority CLI>env>YAML untouched (the `approval`
  block is additive).
- Federate any JWKS IdM (Ory reference), never an IdP; LLM-ops out of scope.
- Documented single-replica constraint (in-memory per-replica `ApprovalManager`).

## First change to apply

`/kbd-apply add-approval-timeout-and-janitor`
