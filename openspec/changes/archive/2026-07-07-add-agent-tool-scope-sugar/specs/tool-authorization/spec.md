# tool-authorization

## ADDED Requirements

### Requirement: Agent tool-scope sugar compiles to validated Cedar
The gateway SHALL provide a per-agent tool-scope config front-end
(`agent_tool_policies`) that compiles to Cedar `permit`/`forbid` policies on
`Action::"call_tool"` for the `Agent::"<agent>"` principal — an ergonomic,
validated shortcut over the policy the engine already runs, never a second policy
authority.

#### Scenario: Allow list authorizes the named tools
- **WHEN** an entry lists a tool under `allow`
- **THEN** that agent is permitted to call that tool and denied any tool not
  covered by an `allow`

#### Scenario: Deny overrides allow
- **WHEN** a tool is matched by both an `allow` and a `deny` (directly or via a
  `deny` glob)
- **THEN** the call is denied — the compiled `forbid` overrides the `permit`

#### Scenario: Glob matches by tool name
- **WHEN** an `allow` or `deny` value contains `*`
- **THEN** it is compiled to a wildcard match against the tool name (e.g.
  `delete_*` blocks every tool whose name begins with `delete_`)

#### Scenario: A policy is scoped to its own agent
- **WHEN** a policy is authored for one agent
- **THEN** it does not grant its allows to any other agent principal

### Requirement: Sugar is injection-safe and fail-closed at load
The gateway SHALL reject a malformed or unsafe `agent_tool_policies` block before
serving, so untrusted text cannot inject Cedar and a bad policy never loads.

#### Scenario: Illegal identifier is rejected
- **WHEN** an entry's agent id or a tool token contains a character outside the
  safe identifier set (e.g. a quote, backslash, whitespace)
- **THEN** the gateway refuses to start rather than interpolating it into Cedar
  source

#### Scenario: Invalid compiled Cedar is rejected
- **WHEN** a sugar entry compiles to Cedar that fails the write-time validator
- **THEN** the gateway refuses to start (fail-closed)

#### Scenario: Empty entry is inert
- **WHEN** an entry has neither `allow` nor `deny`
- **THEN** it compiles to no policy (no accidental broad grant)

#### Scenario: Sugar alongside a database refuses to start
- **WHEN** `agent_tool_policies` is non-empty and a database (the live policy
  authority) is configured
- **THEN** the gateway refuses to start, because config sugar is not yet merged
  into the DB-backed engine and would otherwise be silently unenforced

#### Scenario: Config-only deployment enforces the sugar
- **WHEN** `agent_tool_policies` is non-empty and no database is configured
- **THEN** the compiled, validated policies seed the engine and are enforced
