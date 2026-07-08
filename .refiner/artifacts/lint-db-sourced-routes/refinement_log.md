# Refinement Log — lint-db-sourced-routes

_Change 1/3 of `agent-governance-completeness-and-policy-authoring` (Goal G1 —
extend the agent-governance lint to DB-sourced + hot-reloaded routes)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Lint + reload wiring; no secrets. |
| Never expose the admin server (4457) to public internet | PASS | Unrelated (route governance). |
| Never break existing unit tests without updating them | PASS | 463 core tests green; router refactor behavior-preserving. |
| Never change config priority order (CLI > env > YAML) | PASS | No config-precedence change. |

## Verification gate

- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 463 core + all sub-crates, 0 failed (8 ignored).
- New-code coverage (7 new tests): `merge_routes` surfaces DB-only routes;
  merged-set lint flags a DB-only under-governed agent route (YAML-only lint misses
  it); disabled DB route not linted; reload-decision matrix (strict+findings reject;
  non-strict apply; strict-clean apply); governance-reload-rejected metric renders.
- `config.example.yaml` re-verified to parse.

## Separated security review (security-reviewer agent)

Verdict: **APPROVE — refactor behavior-preserving; reload non-terminating +
fail-closed as intended. No CRITICAL / HIGH / MEDIUM.**

- **Router refactor (INV 1)** — `merge_routes` is a verbatim extraction (compared
  vs `git show HEAD`); `from_config_with_routes` is the original `from_config` body
  with `config.routes` → `route_set`; both `from_config` and
  `from_config_and_db_routes` funnel through it. Merge semantics byte-identical (DB
  wins on id; disabled DB row removes YAML; bad JSON WARN-skipped; compile/sort/
  site-scoping unchanged). No routing regression.
- **Startup lint (INV 2)** — lints the merged set (a DB-only under-governed route
  is caught); `bail!` under strict fires pre-serve; all fallback paths lint what
  they build; single lint call site (no double-WARN).
- **Reload path (INV 3, the key new invariant)** — cannot panic/bail/exit (no
  unwrap/`?` on the live path); strict+findings → early `return` BEFORE the
  router swap (last-good provably retained); non-strict → warn+apply;
  `reload_must_be_rejected(strict, n) = strict && n>0` correct + fully tested.
- **DoS (INV 4)** — `merge_routes` O(n) in route count (operator/admin-governed,
  not request-path attacker input); NOTIFY carries no payload. None.

### LOW-1 — stale docstring on `agent_governance_lint` — FIXED

The doc said DB-route linting was "a follow-up" — but this change IS that
follow-up. Updated to state the gateway lints the merged set at startup + on
reload, and this wrapper is YAML-only-on-purpose.

### LOW-2 — rejected reload only log-grep-able (no metric) — FIXED

Added `flint_governance_reload_rejected_total` (admin metrics) + a render test,
incremented on the strict reload-rejection path, documented in README, and added a
spec scenario ("A rejected reload is observable"). A retain-last-good rejection is
now alertable.

## Outcome

**PASS** — behavior-preserving refactor + non-terminating fail-closed reload
verified by separated review; both LOW findings (stale doc, missing metric) fixed
and tested. Proceed to archive.
