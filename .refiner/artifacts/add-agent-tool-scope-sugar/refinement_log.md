# Refinement Log — add-agent-tool-scope-sugar

_Change 3/3 of `agent-gateway-budget-and-policy-operability` (Goal G3 —
build-004: ergonomic per-agent tool-scope authoring)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Policy compiler; no secrets. |
| Never expose the admin server (4457) to public internet | PASS | Unrelated (config-time policy compilation). |
| Never break existing unit tests without updating them | PASS | 456 core tests green (was 442; +14). |
| Never change config priority order (CLI > env > YAML) | PASS | `agent_tool_policies` is a plain `#[serde(default)]` field. |

## Verification gate

- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 456 core + all sub-crates, 0 failed (8 ignored).
- `config.example.yaml` re-verified to deserialize into `GateConfig` (throwaway
  example, removed after check).
- New-code coverage (14 new tests): 3 config-parse (`config/types.rs`) + 11 sugar
  (`authz/sugar.rs`): allow-only authorizes; deny-overrides-allow; glob-deny
  blocks; glob-allow permits; agent-scoping; empty/illegal-agent rejected;
  injection-safe (`"`-in-id rejected); illegal-tool rejected; empty-entry inert;
  `compile_and_validate` runs the validator; validator rejects Cedar-invalid
  record. The behavior tests drive the REAL `authorize_tool_call` path so
  `context.tool_name` (globs match on it) is populated as on the hot path.

## Separated security review (security-reviewer agent)

Verdict: **injection-safe and fail-closed as intended — no CRITICAL / HIGH /
MEDIUM.** The critical Cedar-injection surface (operator strings → policy SOURCE
text) is fully mitigated:

- **Injection SAFE** — allowlist-charset-before-interpolation. `is_id_char`
  excludes every Cedar-significant character (`"` `\` whitespace `;` `(){}` `/`);
  validation is applied to EVERY interpolated field (agent + every allow tool +
  every deny tool) before any `format!`; the record `id` uses the same restricted
  charset and is only a `PolicyId` prefix, never source. Glob `like` cannot smuggle
  an escape (`\` excluded) and `. : -` are `like`-literals.
- **Fail-closed at load** — `compile_and_validate` runs every emitted record
  through `validate_policy`; `main.rs` refuses to start on any error. The no-DB
  seed path builds only from validated records.
- **Deny-wins REAL**; **no privilege escalation** (principal/action/effect are
  hard-coded template slots — only agent id + tool token are operator-filled);
  **empty entry inert** (no allow-all).

### LOW-1 — silent non-enforcement with a DB attached — FIXED (strengthened to refuse-start)

The reviewer flagged that in a DB-present deployment the sugar was validated but
NOT merged into the engine (a documented follow-up), logged only as a `warn!` — so
an operator relying on a sugar `deny:` as their only control could believe it was
active while it was silently ignored. Although fail-safe (unenforced ≠
over-permissive; DB policy set unchanged), it is a real foot-gun. **Remediation:**
replaced the `warn!` with a **refuse-to-start** — a non-empty `agent_tool_policies`
alongside a database now bails at startup, instructing the operator to author DB
policies (or run config-only). Matches the phase's "hard to misconfigure" ethos and
the other startup posture gates. README + config.example.yaml updated to document
the refusal.

### LOW-2 — validator is a parse gate, not a type gate — NO ACTION

`compile_and_validate` validates with no schema, so `validate_policy` checks
parseability, not schema-type conformance. Consistent with the rest of the engine
(schema is optional), and the charset allowlist already guarantees structural
safety. Accepted as-is (noted).

## Outcome

**PASS** — injection-safe + fail-closed verified; LOW-1 (silent-non-enforcement
foot-gun) strengthened to a refuse-start; LOW-2 accepted as consistent with the
engine. Proceed to archive.
