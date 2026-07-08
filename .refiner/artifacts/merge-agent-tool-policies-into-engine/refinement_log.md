# Refinement Log — merge-agent-tool-policies-into-engine

_Change 2/3 of `agent-governance-completeness-and-policy-authoring` (Goal G2 —
enforce config `agent_tool_policies` sugar alongside DB policies)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Engine overlay + reload wiring; no secrets. |
| Never expose the admin server (4457) to public internet | PASS | Reserved-id guard is admin-router only; no exposure change. |
| Never break existing unit tests without updating them | PASS | 470 core tests green; sugar-id refactor behavior-identical. |
| Never change config priority order (CLI > env > YAML) | PASS | No config-precedence change. |

## Verification gate

- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 470 core + all sub-crates, 0 failed (8 ignored).
- New-code coverage (7 new tests): sugar enforced with DB present; sugar survives a
  policies reload (the core bug fix); precedence matrix (sugar-permit vs DB-forbid →
  deny; DB-permit vs sugar-forbid → deny; two-permit → allow); config-only path;
  reserved-policy-id guard rejects the sugar namespace.
- `config.example.yaml` re-verified to parse.

## Separated security review (security-reviewer agent)

Verdict: **four core claims CONFIRMED CORRECT** — overlay re-applied on EVERY
reload (both `bundle.store` sites concat `self.sugar`; all production reloads
funnel through the single shared `Arc<AuthzEngine>`); deny-wins preserved and
order-independent across sources; fail-closed on DB error (builds from sugar alone,
never fail-open); refuse-start-guard removal introduced no fail-open/panic. The new
tests genuinely prove overlay-survives-reload + the precedence matrix.

### MEDIUM — unenforced PolicyId-namespace invariant → a DB write could silently suppress a sugar `deny:` — FIXED

`concat_records` documented that sugar ids (`agent_tool_sugar::…`) can't collide
with DB row ids, but nothing enforced it. The admin policy API accepted any id, so
a privileged/compromised admin caller could store a policy with id
`agent_tool_sugar::<agent>::<index>`; on the lenient reload path the sugar record's
`PolicySet::add` then errors on the duplicate and the sugar policy is silently
skipped — a vanishing config `deny:` (the exact regression class this change
prevents). Admin-API-gated (not request-path reachable) → MEDIUM, but a silent
removal of a security control. **Remediation:** added `SUGAR_ID_PREFIX` as the
single-source-of-truth const (sugar compiler now uses it in its id `format!`), and
a reserved-namespace guard in `upsert_policy_inner` (via pure
`is_reserved_policy_id`) that **400-rejects** any DB policy write using the prefix —
making the id-disjointness invariant true by construction. Test:
`reserved_policy_id_rejects_sugar_namespace`; spec scenario added ("A stored policy
cannot collide with the sugar namespace").

### LOW — strict-path collision is fail-safe (retain-last-good) — NO ACTION

The reviewer noted the same collision on the strict `reload_from_records` path
fails the whole build and retains last-good (fail-safe), and startup surfaces it as
a start error — only the lenient path silently dropped, which the MEDIUM fix now
prevents at the write boundary. Noted, no change.

## Outcome

**PASS** — overlay-survives-reload + precedence + fail-closed verified by separated
review; the MEDIUM namespace-collision gap (silent `deny:` suppression) fixed at the
admin/DB write boundary with a reserved-prefix guard + test + spec scenario.
Proceed to archive.
