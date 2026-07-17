# Plan — admin-ux-polish-and-diff-view

_Generated: 2026-07-09_
_Backend: OpenSpec_

---

## Sister-Project Sync Audit

Before ordering changes, the following sister projects were inspected for dependency on the APIs or schema added in the prior phase (`cedar-policy-versioning-and-rollback`):

| Project | Status | Integration with flint-gate policies |
|---------|--------|--------------------------------------|
| **flint-forge** | `p15-v1.0-production-readiness` completed; `kbd-reflect` not yet run | Uses `flint_meta.cedar_policies` (own Supabase schema) — no dependency on flint-gate's `authz_policies` or `cedar_policy_versions`. Independent Cedar deployment. **No sync required.** |
| **flint-realtime-fabric** | `phase-27-sovereign-sfu-media-transport-debug` executing (3/4 done) | WebRTC SFU media transport — no flint-gate policy dependency. **No sync required.** |
| **universal-agent-runtime** | `uar-production-ready-uiux-2026-07` executing (change 2/9 done) | Has own Cedar engine (`prometheus-cedar`) for local skill mutations — file-based, not DB-backed. No connection to flint-gate admin API. **No sync required.** |
| **flint-platform-agent** | `a2a-task-catalog-writes` executing | Calls flint-gate admin API via `fpa-gate` Rust adapter; currently only exposes `list_routes` via the `GateAdmin` port trait. Smoke tests use `4457`. **Observation: `GateAdmin` port trait will need policy methods if FPA ever needs to author Cedar policies programmatically** — deferred; not blocking this phase. |
| **flint-gate TypeScript SDK** | `@know-me/flint-gate@0.1.0` — `FlintGateAdmin` class has NO policy methods | The SDK is only used by internal examples (not imported by any external project yet). **Gap noted: SDK needs policy CRUD + history + rollback methods** — adding as Change 5 in this phase (SDK completeness). |

**Sync verdict**: No blocking cross-project dependencies. One proactive SDK gap is added as a non-critical change (Change 5).

---

## Change Order

### Change 1: `add-approvals-auto-polling` (trivial)

**Addresses**: G2 — approvals auto-polling  
**Why first**: Highest operator-safety impact (approvals have expiry timeouts). Trivial scope — ~10 lines in `Approvals.tsx`. No backend changes.  
**Scope**: `web/src/pages/Approvals.tsx` only — add `useEffect` with `setInterval` calling `refetch()` every 5 000 ms, with cleanup on unmount.

### Change 2: `add-written-by-to-policy-list` (medium)

**Addresses**: G4 — `written_by` column in policy list  
**Why second**: Provides attribution for all subsequent policy operations authored during this phase; also wires JWT `sub` into the upsert path so future writes are attributed.  
**Scope**:
- **Rust backend**: Modify `list_policies()` SQL to LEFT JOIN `cedar_policy_versions` on `latest_version` CTE; add `written_by: Option<String>` to `PolicyRow` struct; thread `written_by` from JWT `sub` claim through `upsert_policy_inner`.  
- **TypeScript**: Add `written_by?: string | null` to `PolicyRow` interface (`web/src/api/types.ts`); add "Last by" column to `PolicyTableRow` in `Policies.tsx`.

### Change 3: `add-history-panel-pagination` (small)

**Addresses**: G3 — history panel pagination  
**Why third**: Pure frontend; no backend changes; unblocked after Change 2.  
**Scope**: `PolicyVersionHistory` component in `Policies.tsx` — add `offset`/`hasMore` state, "Load more" button, append-on-load logic. Relies on existing `fetchPolicyHistory(id, offset, limit)` API.

### Change 4: `add-policy-version-diff-view` (medium)

**Addresses**: G1 — policy version diff view  
**Why last**: Requires new npm dependency (`diff@^7`) + most UI work; benefits from Changes 2+3 being in place (attribution and pagination make the diff view more useful).  
**Scope**: Add `diff` npm package; add "Diff" toggle to the version view pane in `PolicyVersionHistory`; render unified diff as colored `<pre>` lines (additions green, deletions red); default diff target: selected version vs. immediately prior version (N vs. N-1).

### Change 5: `add-sdk-policy-methods` (medium, proactive)

**Addresses**: SDK gap — `FlintGateAdmin` missing policy CRUD, history, rollback  
**Why now**: The SDK is unpublished externally; adding these methods while the API surface is fresh and fully known is cheaper than doing it later when external consumers exist.  
**Scope**: `sdks/typescript/src/admin.ts` + `sdks/typescript/src/types.ts` — add `PolicyRow`, `PolicyVersionRow`, `PolicyHistoryResponse`, `RollbackResponse` TS types; add `createPolicy`, `updatePolicy`, `deletePolicy`, `getPolicy`, `listPolicies`, `getPolicyHistory`, `rollbackPolicy` methods to `FlintGateAdmin`. Add tests.

---

## Dependency Map

```
Change 1 (approvals poll)  ──────────────────────────────▶ independent
Change 2 (written_by)      ──────────────────────────────▶ independent
Change 3 (pagination)      ──── needs no backend (but benefits from C2 test env)
Change 4 (diff view)       ──── needs C3 in place (pagination + view pane stable)
Change 5 (SDK)             ──── needs C2 (written_by type matches backend)
```

Changes 1 and 2 can execute in parallel; 3 after 2; 4 after 3; 5 after 2.  
Serial order for KBD single-agent execution: **1 → 2 → 3 → 4 → 5**.

---

## Success Criteria

- [ ] Approvals page auto-refreshes at ≤5 s without manual action.
- [ ] Policy list table shows "Last by" (written_by from latest version JOIN).
- [ ] New upserts populate `written_by` from JWT `sub` (verified via DB or API response).
- [ ] History panel shows "Load more" button when `versions.length === 20`; appends on click.
- [ ] Policy editor shows a diff pane comparing selected version vs. N-1 (or current).
- [ ] SDK `FlintGateAdmin.listPolicies()`, `.getPolicyHistory()`, `.rollbackPolicy()` are implemented and tested.
- [ ] `tsc --noEmit` clean; `vite build` passes; `cargo test --workspace` green; `cargo clippy -D warnings` clean.
