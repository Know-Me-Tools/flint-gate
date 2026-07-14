# Cedar Policies Reference

Flint Gate uses [Cedar](https://www.cedarpolicy.com/) as its authorization
policy language. This page is the definitive reference for writing, validating,
and debugging Cedar policies in flint-gate deployments.

---

## Entity Model

Cedar policies act on entities. Every authorization decision involves a
**principal**, an **action**, and a **resource**. Flint Gate defines the
following entity types:

| Entity type | Id format | Description |
|-------------|-----------|-------------|
| `User` | JWT `sub` claim | A human or system user identified by their JWT |
| `Agent` | JWT `sub` claim | An AI agent; same wire format as User, different type |
| `Service` | JWT `sub` claim | A machine-to-machine service account |
| `Route` | Route id from config | An MCP server tool route (e.g. `my_tool`) |

### Entity examples

```cedar
User::"alice@example.com"
Agent::"my-coding-agent"
Service::"internal-batch-job"
Route::"bash"
Route::"read_file"
Route::"list_directory"
```

### Action

There is currently one action in the flint-gate schema:

| Action | Meaning |
|--------|---------|
| `Action::"call_tool"` | A principal is requesting to call a tool route |

---

## Policy Structure

Cedar policies follow this general shape:

```cedar
// A permit (allow) or forbid (deny) rule
[permit | forbid] (
    principal [== | in] <entity>,
    action    [== | in] [Action::"call_tool"],
    resource  [== | in] <entity>
)
[when { <condition> }]
[unless { <condition> }];
```

### Annotations

Policies may carry Cedar annotations. Flint Gate recognizes one special
annotation:

```cedar
@require_approval("reason string")
permit (
    principal == Agent::"my-agent",
    action == Action::"call_tool",
    resource == Route::"bash"
);
```

`@require_approval("...")` instructs the gateway to gate the request behind a
human-approval flow instead of immediately allowing it. The string argument is
surfaced in the admin UI as the reason an operator sees when reviewing the
approval request.

---

## Common Policy Patterns

### Allow a user to call all tools

```cedar
permit (
    principal == User::"alice@example.com",
    action == Action::"call_tool",
    resource
);
```

### Restrict an agent to specific tools

```cedar
// Allow only safe read operations
permit (
    principal == Agent::"my-coding-agent",
    action == Action::"call_tool",
    resource in [Route::"read_file", Route::"list_directory", Route::"search_files"]
);
```

### Require approval for a destructive tool

```cedar
@require_approval("bash can run arbitrary shell commands — review before allowing")
permit (
    principal == Agent::"my-coding-agent",
    action == Action::"call_tool",
    resource == Route::"bash"
);
```

### Deny a specific principal unconditionally

```cedar
forbid (
    principal == Agent::"untrusted-agent",
    action == Action::"call_tool",
    resource
);
```

> **Fail-closed:** Flint Gate denies by default. A `permit` is required for
> any allowed action. A matching `forbid` always overrides a `permit` —
> `forbid` wins unconditionally.

### Service account with full access

```cedar
permit (
    principal == Service::"internal-batch-job",
    action == Action::"call_tool",
    resource
);
```

---

## Validating Policies

### Via the Admin API

Before writing a policy to the database, you can validate it against the
flint-gate Cedar schema:

```sh
curl -X POST http://localhost:4457/api/policies/validate \
  -H 'Content-Type: application/json' \
  -d '{"policy_text": "permit (principal, action, resource);"}'
```

A valid policy returns `{ "valid": true }`. An invalid policy returns
`{ "valid": false, "errors": ["..."] }`.

### Via the Cedar CLI

If you have the [`cedar`](https://github.com/cedar-policy/cedar) CLI installed:

```sh
cedar validate --schema schema.cedarschema --policies my-policy.cedar
```

The schema file for flint-gate is embedded in the binary (check
`crates/flint-gate-core/src/authz/schema.cedarschema` in the source tree).

---

## Debugging

### Enable verbose authorization logging

```sh
RUST_LOG=flint_gate_core::authz=debug ./flint-gate
```

Every Cedar evaluation emits a log line with:

```
DEBUG authz decision principal="User::alice" action="call_tool" resource="Route::bash" decision=Allow matched_policy="policy-abc123"
```

### Query the audit table

Every authorization decision is persisted to `authz_audit`. To see recent
decisions for a specific principal:

```sql
SELECT decision, resource_id, policy_id, created_at
FROM authz_audit
WHERE principal_id = 'my-agent'
ORDER BY created_at DESC
LIMIT 20;
```

See [Operations Runbook — Audit Trail](./operations.md#audit-trail) for the
full schema and common query patterns.

### Common errors

| Error | Likely cause |
|-------|--------------|
| `ParseError: unexpected token` | Syntax error in the policy text — check Cedar syntax |
| `SchemaError: unknown entity type` | Used an entity type not in the schema (e.g. `Group`) |
| `SchemaError: unknown action` | Used an action other than `Action::"call_tool"` |
| Decision: `Deny` with no `policy_id` | No permit matched — add a permit policy |
| Decision: `Deny` with a `policy_id` | A `forbid` policy matched and overrode a permit |

---

## Hot-Reload

Policy changes take effect without restarting the gateway. When a policy is
created, updated, or deleted via the admin API (or directly in the database),
the gateway receives a Postgres NOTIFY event and atomically swaps in the new
Cedar bundle within seconds.

The swap is parse-before-swap: if the new policy set fails to compile, the
previous bundle remains authoritative and a `WARN` is logged. No traffic is
disrupted on a bad policy update.

See the `AuthzEngine` doc comment in
`crates/flint-gate-core/src/authz/engine.rs` for the full atomicity guarantee.
