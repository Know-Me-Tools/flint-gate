# Decision Log — agent-approval-and-step-up-flows

### 2026-07-08 — Build-vs-adopt (all 3 goals) + scope correction
Decision: BUILD/wire in-tree; ZERO new dependencies; the approval flow is ALREADY built (Assess) so the phase is G3 (fail-closed lifecycle) + G2 (operator surface) + G1 (verify/comments) | Provenance: research (Assess trace + in-tree tokio patterns)
Rationale: engine/ApprovalManager/stream-pause/admin-CRUD all exist; a HITL crate would duplicate working code. tokio::time already used in the same pipeline file.

### 2026-07-08 — D1 G3 paused-stream timeout = auto-DENY (safety-critical)
Decision: add a sleep_until(nearest expires_at) arm to the paused-stream select! (pipeline.rs:815-833); on fire, resolve the held call as Deny (emit deny event + resume to termination), NOT silent drop | Provenance: Assess (recv() at pipeline.rs:821 hangs forever) + in-tree watchdog interval (pipeline.rs:744)
Rationale: auto-deny is the fail-closed analog for an undecided approval — never silent-allow, never hang; mirrors the existing cancel arm; reuse the monotonic expires_at Instant.

### 2026-07-08 — D2 G3 purge_expired janitor
Decision: spawn a background tokio::time::interval task (mirror the watchdog) calling purge_expired() periodically, started in main.rs | Provenance: Assess (purge_expired defined but never called)
Rationale: closes the DashMap leak; hygiene for streams that already ended (distinct lifecycle from the D1 stream-timeout correctness fix).

### 2026-07-08 — D3 G3 approval config block
Decision: add approval:{enabled:bool=true, ttl_seconds:Option<u64>} to config/types.rs (serde default); enabled:false -> RequireApproval fails closed to Deny | Provenance: Assess (no approval config; 300s hardcoded)
Rationale: config-driven fail-safe consistent with the phase-line; enabled is a fail-closed kill-switch. Per-route approval config is a follow-up.

### 2026-07-08 — D4 G2 list endpoint + iterate method
Decision: add ApprovalManager::list()->Vec<ApprovalStatus> (skip expired) + GET /approvals + GET /approvals/{id} on the admin router | Provenance: Assess (only single-id status; len/is_empty are cfg(test); no GET route)
Rationale: minimal backing method unblocks the operator surface; GET mirrors existing list handlers; ApprovalStatus already Serialize.

### 2026-07-08 — D5 G2 web UI = new Approvals tab with poll refresh
Decision: new /approvals tab (App.tsx route+nav, pages/Approvals.tsx, client fns/hooks/types) following AgentIdentities/Policies pattern; TanStack Query refetchInterval poll; approve/deny -> POST /approvals/{id}/decision | Provenance: Assess (no UI) + existing React kit
Rationale: pending approvals are time-sensitive (expire) so a poll is justified; a new tab matches one-surface-per-governance-concern IA.

### 2026-07-08 — D6 multi-replica reachability = documented single-replica constraint
Decision: document a single-replica constraint this phase (list/decide see only the local replica's pending approvals); shared store + sticky routing is a follow-up | Provenance: Assess (ApprovalManager is in-memory per-replica DashMap)
Rationale: matches the phase-line's documented-constraint discipline (sugar-overlay, budget windows); cross-replica approval routing is a materially larger effort. MUST be loudly documented (README + config).

### 2026-07-08 — D7 G1 in-band client decision channel = OUT of scope
Decision: the emitted approval-request event + Admin REST decision + new UI is a complete operator flow; an in-band client-decides-over-the-stream path is deferred | Provenance: Assess
Rationale: keeps G1 to comment-fix + verification; in-band is a larger stream-protocol change and a separate concern (client-decides vs operator-decides).
