# Plan — agent-governance-completeness-and-policy-authoring

_Planned 2026-07-07. Backend: openspec · Driver: /kbd-apply (one task/turn).
3 changes, dependency-ordered. Zero new dependencies (from
`library-candidates.json` — all three are `build_required`, no adopt/adapt)._

## Ordered change list

| Order | Change | Goal | Depends on | Tasks | Recommended reviewer |
|-------|--------|------|-----------|-------|----------------------|
| 1 | `lint-db-sourced-routes` | G1 | — | 5 | security-reviewer (reload error model + fail-closed) |
| 2 | `merge-agent-tool-policies-into-engine` | G2 | — (ordered after 1) | 5 | security-reviewer (merge/reload fail-closed + precedence) |
| 3 | `admin-tool-scope-builder` | G3 | **change 2** | 5 | security-reviewer (**API-boundary injection re-check — highest priority**) |

## Ordering rationale

- **G1 first** — independent of the others, and the phase's HIGH-first item
  (closes the sharpest remaining "silent under-application" edge: DB/hot-reloaded
  agent routes escaping `strict_agent_governance`). No dependency, so it de-risks
  the reload-error-model design (reject-and-retain on a live process) before the
  engine-merge work.
- **G2 second** — independent of G1 in code, but ordered after it so each change
  lands and is reviewed in isolation. G2 removes the refuse-start guard and makes
  the sugar enforce alongside DB policies — it MUST precede G3.
- **G3 last** — **hard dependency on G2**: the admin builder writes into the
  merged/enforced sugar-overlay set G2 establishes. Building G3 before G2 would
  ship a UI that authors an unenforced overlay (the exact foot-gun G2 fixes).

## Per-change reuse annotations (from library-candidates.json → build_required)

- **`lint-db-sourced-routes`** reuses: `config/types.rs:157`
  `agent_governance_lint_routes(&[RouteConfig])` (lints any slice);
  `proxy/router.rs:128` (merged set already built, discarded);
  `cache/mod.rs:398` `rebuild_router_from_db` (reload point with config+db_routes).
- **`merge-agent-tool-policies-into-engine`** reuses: `authz/sugar.rs:139`
  `compile_and_validate`; `authz/bundle.rs:91` `CedarBundle::from_records` (merges
  any `&[PolicyRecord]`); the reload call sites `engine.rs:229`,
  `cache/mod.rs:372`, `admin/mod.rs:727`.
- **`admin-tool-scope-builder`** reuses: `authz/sugar.rs:72`
  `compile_agent_tool_policies` + its allowlist-charset injection defense;
  `admin/mod.rs:680` `upsert_policy_inner` (validate→persist→reload pattern);
  `web/src/pages/AgentIdentities.tsx` + `web/src/pages/Routes.tsx` (UI templates).

## Per-change QA gate (uniform)

Each change, on reaching 5/5 tasks:
1. **artifact-refiner** constraint validation → `.refiner/artifacts/<id>/refinement_log.md`.
2. **separated security-reviewer** (author never grades its own authz/policy/reload
   seam) — full review on every change; **G3 requires an explicit Cedar
   string-concatenation injection re-check at the admin-API boundary**.
3. A **fail-closed / fail-safe test** on every new path (G1 reject-and-retain; G2
   skip-on-error merge + deny-wins matrix; G3 illegal-input → 400).
4. `cargo clippy --workspace -- -D warnings` + `cargo test --workspace` green;
   ≥80% new-code coverage; web build green (G3).
5. Archive via `openspec archive <id> --skip-specs --yes`; sync `progress.json` +
   advance waypoint in SEPARATE commands (pipeline-enforce hook).

## Constraints carried (from .kbd-orchestrator/constraints.md)

- No secrets / signing keys / prod DB creds committed.
- Admin server (4457) never public — the new G3 endpoint is admin-router only.
- No broken existing tests; config priority CLI>env>YAML untouched.
- Federate any JWKS IdM (Ory reference), never an IdP; LLM-ops out of scope.

## First change to apply

`/kbd-apply lint-db-sourced-routes`
