# Execution — agent-gateway-hardening-and-exposure

_Generated: 2026-07-04 · Backend selected: **openspec** · Driver: **kbd-apply** (one task/turn)_

## Backend selection

| Field | Value | Why |
|-------|-------|-----|
| Backend | **openspec** | Available; spec-backed traceability; matches the two prior phases' flow |
| Driver | **kbd-apply** | KBD-owned per-task loop — fires KBD hooks, syncs `progress.json` + waypoint, emits position signals. **Never** bare `/opsx:apply` |
| Execution agent | Claude Code (this session) | Rust/axum; per-change review via rust-reviewer + security-reviewer |

## Dispatch contract

Drive each change **one task per turn** via the KBD apply driver:

```sh
export KBD_ORCHESTRATOR_ROOT="$HOME/.claude"
APPLY="$HOME/.claude/skills/kbd-apply/kbd-apply.sh"
CHANGE="add-oauth-endpoint-hardening"   # then add-bcrypt-secrets, add-identity-classification-edges, add-actor-token-and-hydra-delegate

"$APPLY" list "$CHANGE"
"$APPLY" begin-task "$CHANGE" "$ID" "$I" "$TOTAL" "$T"   # fires task:before + position signal
#   … implement EXACTLY this one task (code/tests/docs) …
"$APPLY" end-task   "$CHANGE" "$ID" "$I" "$TOTAL" "$T"   # marks done + syncs + fires task:after
# after the last task → artifact-refiner QA + security-review → verify → archive (--skip-specs)
```

## Change queue (dependency order)

| # | Change | Goal | Tasks | Status |
|---|--------|------|-------|--------|
| 1 | `add-oauth-endpoint-hardening` | G1 | 5 | **pending — apply next** |
| 2 | `add-bcrypt-secrets` | G2 | 5 | pending (after 1) |
| 3 | `add-identity-classification-edges` | G3 | 5 | pending (after 1–2) |
| 4 | `add-actor-token-and-hydra-delegate` | G4 | 4 | pending (after 1–2) |

## Per-change QA gate (mandatory — all 4 touch auth)

After a change reaches all-tasks-done:
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<change-id>/refinement_log.md`
2. **security-review** (every change hardens an auth/secret path; the two-phase
   lesson: auth degrades to fail-open through config/seam gaps — audit **every**
   authorize entry point when touched)
3. Confirm the change's **`degrades_to_deny` fail-closed test** passes
4. Workspace gate: `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`,
   `cargo test --workspace`; ≥80% new-code coverage
5. Archive: `openspec archive <change> --skip-specs --yes`, then sync `progress.json` + waypoint

QA is **not** skippable here (each change modifies ≥3 files and is security-sensitive).

## Guardrails (carried from Analyze/Plan)

- **Federate any JWKS IdM; Ory is standard; never an IdP.**
- **Fail-closed everywhere** + `degrades_to_deny` tests.
- **One deliberate new crate (`bcrypt@0.19`)** in change 2; otherwise reuse.
- **jsonwebtoken@9 pin, single-binary, no sidecar.**
- **RFC-mandated auth is a MUST** (G1 introspection auth per RFC 7662 §2.1).

## First action

`/kbd-apply add-oauth-endpoint-hardening`
