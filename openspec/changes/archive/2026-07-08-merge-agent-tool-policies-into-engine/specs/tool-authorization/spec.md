# tool-authorization

## MODIFIED Requirements

### Requirement: Config tool-scope sugar enforces alongside database policies
The gateway SHALL compile `agent_tool_policies` into the live authorization engine
**alongside** database-stored policies, so the sugar is enforced in a
database-backed deployment rather than refused at startup.

#### Scenario: Sugar is enforced with a database present
- **WHEN** `agent_tool_policies` is non-empty and a database is configured
- **THEN** the gateway starts and the compiled sugar policies are evaluated
  together with the database policies (the previous refuse-to-start is removed)

#### Scenario: Sugar survives a policies reload
- **WHEN** a policies hot-reload or an admin policy CRUD operation rebuilds the
  engine from the database
- **THEN** the config sugar overlay is preserved (re-applied), not dropped

#### Scenario: Cross-source deny wins
- **WHEN** a database policy `forbid`s a tool that a sugar policy `permit`s (or
  vice-versa) for the same agent
- **THEN** the call is denied — Cedar `forbid` overrides `permit` regardless of
  which source contributed each policy

#### Scenario: Config-only deployment still enforces the sugar
- **WHEN** no database is configured and `agent_tool_policies` is non-empty
- **THEN** the compiled, validated sugar seeds the engine and is enforced (the
  prior behavior is preserved)

#### Scenario: A malformed merged policy cannot open the gate
- **WHEN** the merged (database + sugar) policy set contains a policy that fails
  to parse
- **THEN** that policy is skipped and the last-good bundle is retained (Cedar
  skip-on-error + default-deny); the gate never opens on error

#### Scenario: A stored policy cannot collide with the sugar namespace
- **WHEN** an admin policy write uses an id in the reserved compiled-sugar
  namespace (the `agent_tool_sugar::` prefix)
- **THEN** the write is rejected with 400, so a database row can never collide
  with (and silently suppress) a config sugar overlay policy
