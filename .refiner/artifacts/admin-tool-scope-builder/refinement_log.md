# Refinement Log — admin-tool-scope-builder

_Change 3/3 of `agent-governance-completeness-and-policy-authoring` (Goal G3 —
admin-UI + endpoint to author agent tool-scopes)._
_QA gate: artifact-refiner constraint validation + separated security review with
an explicit API-boundary injection re-check (the phase's highest-priority security
item)._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Endpoint + UI; no secrets. |
| Never expose the admin server (4457) to public internet | PASS | `/tool-scopes` is admin-router only; existing exposure posture unchanged. |
| Never break existing unit tests without updating them | PASS | 475 core tests green; web typecheck + build green. |
| Never change config priority order (CLI > env > YAML) | PASS | No config-precedence change. |

## Verification gate

- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 475 core + all sub-crates, 0 failed (8 ignored).
- Web: `pnpm typecheck` clean; `pnpm build` green (2304 modules).
- New-code coverage (5 backend tests): valid → correct `tool_scope::<agent>` id +
  compiled `permit`/`forbid`; illegal agent (`ci"bot`) → 400; illegal tool → 400;
  empty → 400; **no-raw-Cedar-bypass** (an injected `policy_text` field is dropped;
  only structured fields compile).

## Separated security review (security-reviewer agent)

Verdict: **PASS — no CRITICAL / HIGH / MEDIUM.** The mandated deliverable is
explicit: **an attacker with admin-API access CANNOT inject arbitrary Cedar via
this endpoint.**

- **Injection boundary closed by construction** — `ToolScopeRequest` is
  `{agent, allow, deny}` only (no `policy_text`/schema/entities); serde drops
  unknown fields, so a raw-Cedar body is inert (proven by
  `tool_scope_request_has_no_raw_cedar_field`). The ONLY route from HTTP input to
  Cedar source is `compile_tool_scope` → `compile_and_validate`, whose
  allowlist-charset check rejects quotes/backslashes/whitespace/`;`/`(){}` BEFORE
  any `format!` into Cedar, then a real `PolicySet::from_str` parse runs as
  defense in depth. The breakout vectors (`ci"bot`,
  `a) ; permit(principal,action,resource); //`) fail closed 400.
- **Safe id derivation** — `tool_scope::<agent>` is derived only from the
  already-validated agent; can't collide with the reserved `agent_tool_sugar::`
  namespace; parameterized `ON CONFLICT (id)` upsert (no SQLi); exact-match id
  means an attacker can only upsert their own agent's row, not overwrite arbitrary
  policies. `delete_tool_scope` always prepends the prefix → can only delete within
  the `tool_scope::` namespace.
- **Admin-only + DB-gated; fail-closed; no new panic** — routes on the loopback
  admin router; `db_not_configured()` without a DB; compile → 400, DB → 500,
  reload-fail → 500; no unwrap/`?` on the request path.
- **Deny-wins persisted**; **web UI** posts only `{agent,allow,deny}`, `parseToolList`
  splits safely, `policy_text` rendered as auto-escaped JSX text (no
  `dangerouslySetInnerHTML`), `encodeURIComponent` on the delete path.

### LOW observations — NO ACTION (reviewer-confirmed)

- Glob `*` → Cedar `like` wildcard is expected behavior, not injection.
- `internal_error` surfaces DB error strings in the 500 body — mirrors the existing
  policy handlers and is only reachable on the authenticated loopback admin surface.

## Outcome

**PASS** — the new admin authoring surface is injection-safe at the API boundary
(structured-only, no raw-Cedar path), admin-only, fail-closed, deny-wins-preserving,
and free of web XSS. No code changes required post-review. Proceed to archive.
