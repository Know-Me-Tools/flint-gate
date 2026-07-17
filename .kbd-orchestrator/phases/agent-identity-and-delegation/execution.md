# Execution — agent-identity-and-delegation

_Generated: 2026-07-04 · Backend selected: **openspec** · Driver: **kbd-apply** (one task/turn)_

## Backend selection

| Field | Value | Why |
|-------|-------|-----|
| Backend | **openspec** | OpenSpec is available; spec-backed traceability; matches the prior phase's proven flow |
| Driver | **kbd-apply** | KBD-owned per-task loop — fires KBD hooks, syncs `progress.json` + waypoint, emits position signals. **Never** bare `/opsx:apply` |
| Execution agent | Claude Code (this session) | Rust/axum + TS UI work; per-change review via rust-reviewer + security-reviewer |

## Dispatch contract

Drive each change **one task per turn** via the KBD apply driver:

```sh
export KBD_ORCHESTRATOR_ROOT="$HOME/.claude"
APPLY="$KBD_ORCHESTRATOR_ROOT/skills/kbd-apply/kbd-apply.sh"   # or ~/.claude/skills/kbd-apply/kbd-apply.sh
CHANGE="add-admin-authn"        # then add-token-exchange, add-client-creds-introspection, add-nhi-cedar-principals

"$APPLY" list "$CHANGE"                                  # task surface (TSV)
"$APPLY" begin-task "$CHANGE" "$ID" "$I" "$TOTAL" "$T"   # fires task:before + position signal
#   … implement EXACTLY this one task (code/tests/docs) …
"$APPLY" end-task   "$CHANGE" "$ID" "$I" "$TOTAL" "$T"   # marks done + syncs + fires task:after
# after the last task → artifact-refiner QA → verify → archive (--skip-specs)
```

## Change queue (dependency order)

| # | Change | Goal | Tasks | Status |
|---|--------|------|-------|--------|
| 1 | `add-admin-authn` | G1 | 5 | **pending — apply next** |
| 2 | `add-token-exchange` | G2 | 6 | pending (after 1) |
| 3 | `add-client-creds-introspection` | G3 | 6 | pending (after 2) |
| 4 | `add-nhi-cedar-principals` | G4 | 6 | pending (after 3) |

## Per-change QA gate (mandatory — all 4 touch authz/identity)

After a change reaches all-tasks-done:
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<change-id>/refinement_log.md`
2. **security-review** (every change is auth-sensitive; prior-phase lesson: auth code fails *open* by default)
3. Confirm each change's **`degrades_to_deny` fail-closed test** passes
4. Workspace gate: `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`; ≥80% coverage on new code
5. Archive: `openspec archive <change> --skip-specs --yes` (code-delta change, no capability spec), then sync `progress.json` + waypoint

QA is **not** skippable here (each change modifies ≥3 files and is security-sensitive).

## Guardrails (carried from Analyze/Plan)

- **Federate any JWT-capable IdM; Ory is standard.** No hard dependency on Ory-specific behavior.
- **Fail-closed everywhere** — the recurring prior-phase defect.
- **Zero new crates** — all reuse (`library-candidates.json`).
- **jsonwebtoken@9 pin, single-binary, no sidecar.**

## First action

`/kbd-apply add-admin-authn`
