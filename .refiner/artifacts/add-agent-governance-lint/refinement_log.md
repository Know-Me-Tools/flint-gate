# Refinement Log — add-agent-governance-lint

_Change 1/3 of `agent-gateway-budget-and-policy-operability` (Goal G1 + G4)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Config lint only; no secrets. |
| Never expose the admin server (4457) to public internet | PASS | Unrelated (startup config lint). |
| Never break existing unit tests without updating them | PASS | 430 core tests green. |
| Never change config priority order (CLI > env > YAML) | PASS | Adds `server.strict_agent_governance` field only. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean (fixed a
  doc-list-indentation lint on the new doc comment).
- `cargo test --workspace` — 430 core tests, 0 failed (8 ignored).
  (A transient `No space left on device` during linking was resolved by cleaning
  ~12 GB of stale `target/debug/incremental` — an environment issue, not code.)
- New-code coverage: reachable/unreachable provider matrix, route→site fallback,
  clean-config-empty, lifetime+agent, unresolvable-provider, dedup, and the
  explicit-route-set helper (9 lint tests).

## Separated security review (security-reviewer agent)

Verdict: **lint logic correct, advisory-only (no runtime weakening)** — the
`Jwt|Mcp` reachability set is complete for the runtime agent definition; enabled-
only scope, budget logic, strict-bail, and no-panic all confirmed. Findings:

### M1 — DB-sourced routes not linted — MITIGATED (reusable + documented)

The startup lint walks only YAML routes; DB routes (`database.override_yaml`) +
hot-reload serve un-linted agent surfaces. Fully wiring the lint at DB-route
load/reload is beyond this change's scope. **Remediation:** split the lint into
`agent_governance_lint_routes(&[RouteConfig])` (public, reusable) so a follow-up
can lint the merged/DB route set in one line; the doc guarantee is scoped to YAML
with an explicit follow-up note (README + fn doc). Test:
`lint_routes_helper_lints_an_explicit_route_set`. Tracked as phase debt.

### M2 — no tests — NOT APPLICABLE (already resolved)

The review grepped the base version; task 4 added 7 lint tests before the review
ran. Now 9 (with the two review-fix tests).

### L1 — unresolvable/typo'd provider silently non-agent — FIXED

Added `GovernanceReason::UnresolvableAuthProvider`: a route naming an undefined
provider is now flagged (it 500s at runtime — surfaced at startup instead). Test:
`lint_flags_unresolvable_auth_provider`.

### L2 — duplicate findings — FIXED

Findings deduped per `(route_id, reason)` via a `HashSet`. Test:
`lint_deduplicates_findings_per_route_and_reason`.

### Confirmed correct (no action)

`Jwt|Mcp` reachability complete; `auth:None` correctly excluded (anonymous→User
at runtime); enabled-only correct; strict-bail fail-safe on ANY finding; lint is
pure/advisory (runtime authz+budget still enforce, incl. the principal-kind
defense-in-depth); no panics on hostile config.

## Outcome

**PASS** — lint logic verified correct + advisory-only; the M1 DB-route gap is
mitigated (reusable helper + scoped doc + tracked debt); L1/L2 fixed and tested;
M2 was already resolved. Proceed to archive.
