# admin-tool-scope-builder

**Phase:** agent-governance-completeness-and-policy-authoring · **Goal:** G3 (build-003)
**Scope:** `crates/flint-gate-core/src/admin/mod.rs` (new tool-scope admin
endpoint), `web/src/` (Policies-tab tool-scope builder UI + client fn + hook), docs.
**Depends on:** `merge-agent-tool-policies-into-engine` (G2 — the UI must write into
the merged/enforced set, not an unenforced overlay).

## Why

The allow/deny → Cedar compiler (`authz/sugar.rs`) exists but is **YAML-only** —
no admin route, no client fn, no UI (grep confirms zero matches in `admin/` +
`web/src/`). The admin Policies page accepts only **raw Cedar** text. Operators
have no ergonomic, in-product way to author agent tool-scopes — the deferred G3
ergonomics gap.

## What

1. **New admin endpoint** accepting a structured `{ agent, allow[], deny[] }`
   body, compiling it via **`compile_and_validate`** (the SAME allowlist-charset +
   `validate_policy` 400-gate the config path uses), and persisting it as the
   sugar-overlay source (aligned with G2's overlay model), then reloading. Mirrors
   the `/policies` `upsert_policy_inner` validate-then-reload pattern.
2. **Policies-tab builder UI** — a structured form (agent id, allow-list, deny-list
   with glob support) following the **AgentIdentities.tsx** (modal builder + RQ
   hooks) and **Routes.tsx** (nested structured config) patterns, plus a client fn
   in `web/src/api/admin.ts` and a React-Query hook.

## Non-goals

- A raw-Cedar path for tool-scopes — the endpoint compiles ONLY from the structured
  `{agent,allow,deny}` (the raw-Cedar `/policies` page stays for advanced use).
- New compiler semantics — reuse `compile_agent_tool_policies`.
- Bulk import / policy templates beyond the single-agent builder.

## Fail-safe requirement (SECURITY-CRITICAL)

An admin endpoint makes the sugar's operator input **attacker-adjacent** (a hostile
or compromised admin session). The endpoint MUST route input **only** through the
existing allowlist-charset `compile_and_validate` — never a looser or raw-Cedar
path — so the Cedar string-concatenation injection the compiler already defends
(last phase's security review) cannot be reached. An illegal agent id / tool token
is rejected 400, fail-closed, exactly as at config load. **Separated security
review with an explicit injection re-check at the API boundary is required.**

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage on the new endpoint (valid compiles+persists+reloads; illegal
agent/tool → 400; deny-wins preserved via the compiler); web build green; the
builder posts and round-trips. Separated security review (injection at the boundary).
