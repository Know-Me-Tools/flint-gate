# add-agent-tool-scope-sugar

**Phase:** agent-gateway-budget-and-policy-operability · **Goal:** G3 (build-004)
**Scope:** `crates/flint-gate-core/src/config/types.rs` (sugar schema),
`crates/flint-gate-core/src/authz/` (sugar→Cedar compiler + validation wiring), docs.
**Depends on:** nothing (independent of 1–2).

## Why

Agent tool-scoping today requires hand-written raw Cedar — an operator must author
`permit(principal == Agent::"ci-bot", action == Action::"call_tool", resource ==
Route::"deploy")` per tool. That is error-prone and the deferred G1 ergonomics gap.

## What

Add a per-agent config sugar block — e.g.:

```yaml
agent_tool_policies:
  - agent: "ci-bot"
    allow: ["deploy", "run_tests"]
    deny: ["delete_*"]
```

that **compiles to Cedar** `permit`/`forbid` policies on `Action::"call_tool"` +
`Route::"<tool>"` for the `Agent::"<agent>"` principal, then runs through the
**existing write-time validator** (`authz/validator.rs`) before load — so the sugar
is a validated front-end over the Cedar the engine already runs, not a new policy
authority (federate/validate, never a second engine).

## Non-goals

- **Admin-UI policy builder — DEFERRED** (config sugar this phase; UI is a
  separable follow-up).
- New Cedar semantics — the sugar only emits `call_tool`/`Route` policies the
  engine already understands.
- `User`/`Service` sugar (agent tool-scoping is the target; extendable later).

## Fail-safe requirement

Sugar that compiles to invalid Cedar is **rejected at load** by the existing
validator (fail-closed — a bad policy never loads). `deny` wins over `allow`
(Cedar `forbid` overrides `permit`). Tested: allow-only, deny-override, glob
(`delete_*`), and an invalid entry rejected.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; sugar→Cedar compilation + validation tests (allow/deny-override/
glob/reject-invalid).
