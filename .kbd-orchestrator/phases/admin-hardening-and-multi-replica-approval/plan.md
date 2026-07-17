# Plan — admin-hardening-and-multi-replica-approval

_Planned 2026-07-08. Backend: openspec · Driver: /kbd-apply (one task/turn).
3 changes, dependency-ordered. Zero new dependencies — `tower_governor` and
`CredentialKeyExtractor` are already in `Cargo.toml` (Assess confirmed).
Analyze phase skipped: all decisions closed by code inspection during Assess._

## Ordered change list

| Order | Change | Goal(s) | Depends on | Tasks | Recommended reviewer |
|-------|--------|---------|-----------|-------|----------------------|
| 1 | `add-admin-write-rate-limiting` | G1 | — (independent) | 5 | security-reviewer (rate-limit bypass paths; credential-key extractor; public-probe exemption) |
| 2 | `add-approval-cap-and-janitor-config` | G2 | — (independent) | 5 | security-reviewer (fail-closed on CapExceeded — no silent-allow path) |
| 3 | `add-admin-hardening-and-multi-replica-doc` | G3 + G4 | — (independent; last for coherent README) | 5 | security-reviewer (body-size cap bypass; multi-replica proof-of-constraint test) |

## Ordering rationale

All three changes are independent (no data-structure dependency between them),
so the order is by descending security impact:

- **Change 1 first** — closes HIGH-2 (admin write rate-limit), the most
  operationally significant security gap; it is a pure wiring change with the
  lowest defect risk (infra is already proven on the proxy/OAuth routers).
- **Change 2 second** — closes MEDIUM-1 (unbounded DashMap cap); requires
  a new error variant and a short fail-closed propagation chain. Independent
  of change 1 (different subsystems).
- **Change 3 last** — G3 + G4 hardening (body-size cap, multi-replica
  warning, CORS warning, cross-replica integration test + README); cleanest
  to land after the subsystem changes since it touches `main.rs` for the
  startup warnings and `README.md` for the final multi-replica note.

## Per-change reuse annotations (all build_required)

- **`add-admin-write-rate-limiting`** reuses: `ratelimit::build_governor_layer`
  + `CredentialKeyExtractor` (already in `src/ratelimit/governor_layer.rs`);
  `admin_router_with_auth` in `admin/mod.rs`; `ServerConfig` in `config/types.rs`;
  `RateLimitConfig` (already defined, same struct); `main.rs` proxy/OAuth
  rate-limit wiring pattern (lines 660-672).
- **`add-approval-cap-and-janitor-config`** reuses: `ApprovalConfig` in
  `config/types.rs`; `ApprovalManager` + `ApprovalError` in `approval/mod.rs`;
  `middleware/pipeline.rs` register-error branch; `main.rs` janitor spawn
  (lines 362-381).
- **`add-admin-hardening-and-multi-replica-doc`** reuses: `admin_router_with_auth`
  protected sub-router (apply `DefaultBodyLimit::max` layer); `admin/mod.rs`
  test harness; `approval/mod.rs` (`ApprovalManager::new` + `register` + `decide`
  for the cross-replica test); `main.rs` startup-check pattern.

## Per-change QA gate (uniform)

Each change, on reaching its last task:
1. **Separated security-reviewer** on all three changes (admin rate-limit bypass
   paths; CapExceeded fail-closed; body-size cap; multi-replica proof test).
2. A **fail-closed / fail-safe test** on every new path:
   - Change 1: 429 on burst; `/health` bypass; `None` layer → no restriction.
   - Change 2: CapExceeded → Deny (not panic); cap enforced; janitor config used.
   - Change 3: 413 on oversized body; NotFound on cross-replica decide.
3. `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` green.
4. Archive via `openspec archive <id> --skip-specs --yes`; sync `progress.json`
   + advance waypoint in SEPARATE commands (pipeline-enforce hook).

## Constraints carried (from .kbd-orchestrator/constraints.md)

- Admin server (4457) never public — the rate-limit config and body-size cap
  MUST NOT relax the loopback-enforcement posture.
- No secrets / signing keys / prod DB creds committed.
- No broken existing tests; config priority CLI>env>YAML untouched (all new
  fields are additive with serde defaults).
- Federate any JWKS IdM (Ory reference), never an IdP.
- Fail-closed: CapExceeded → Deny, never silent-allow; rate-limit 429 is
  a rejection, not a hang.

## First change to apply

`/kbd-apply add-admin-write-rate-limiting`
