# add-policy-rollback-endpoint

## Summary

Add `POST /policies/{id}/rollback` to the admin API. Given a `version_num`, the endpoint restores the versioned policy text into `authz_policies`, triggers a Cedar hot-reload, and returns a structured response. Invalid rollback targets (missing version, invalid Cedar policy text) are rejected — fail-closed.

## Motivation

The version history is only useful if operators can act on it. This endpoint closes the reversibility loop: validate → simulate → write → observe reload → **rollback if needed**.

## Design

### Route

`.route("/policies/{id}/rollback", post(rollback_policy_handler))` — registered before `/{id}` wildcard (alongside `history`).

### Request body

```json
{ "version_num": 2 }
```

### Response

**200 OK** (success):
```json
{
  "status": "rolled_back",
  "policy_id": "my-policy",
  "from_version": 3,
  "to_version": 4,
  "reloaded": true
}
```

`to_version` is the newly-written version number (rollback itself creates a new version entry — the undo is auditable).

**404** — policy or version not found.

**422** — the requested version's `policy_text` fails Cedar validation (fail-closed: never write an invalid policy). Response includes the Cedar parse errors.

**503** — DB not configured.

### Handler logic

```
1. Parse body → RollbackRequest { version_num: i32 }
2. Fetch the target version row via db.get_policy_version(id, version_num) → 404 if None
3. Fetch the current version for from_version via db.list_policy_versions(id, 0, 1) → current_version_num
4. Cedar-validate the restored text:
       validate_policy(&PolicyRecord { id, policy_text: row.policy_text, ... })
       → 422 with error details if invalid
5. Upsert via db.upsert_policy(id, row.policy_text, row.schema_json, row.entities_json, enabled=true, written_by=None)
       → this also inserts a new cedar_policy_versions row (to_version)
6. Reload: state.authz.reload_from_database(db).await → 500 if fails
7. Return 200 with { status, policy_id, from_version, to_version, reloaded: true }
```

The rollback reuses `upsert_policy_inner`'s validation logic directly (imports `validate_policy` already in scope) rather than calling the HTTP handler — avoids double-HTTP and keeps the Cedar validation path consistent.

### Structs

```rust
#[derive(Deserialize)]
struct RollbackRequest {
    version_num: i32,
}

#[derive(Serialize)]
struct RollbackResponse {
    status: String,       // "rolled_back"
    policy_id: String,
    from_version: i32,
    to_version: i32,
    reloaded: bool,
}
```

### Fail-closed invariant

A version row inserted when Cedar validation was less strict may fail current validation. The 422 response includes the Cedar errors so the operator knows why the rollback was rejected. They can either roll back to a different (valid) version or edit the policy directly.

## Tasks
- [ ] Add `RollbackRequest` and `RollbackResponse` structs in `admin/mod.rs`
- [ ] Add `rollback_policy_handler`: fetch version (404), Cedar-validate (422 with errors), upsert + get new version_num, reload, return `RollbackResponse`
- [ ] Register `.route("/policies/{id}/rollback", post(rollback_policy_handler))` before `/{id}`
- [ ] Add tests: 404 on unknown policy; 404 on unknown version_num; 422 when version row has invalid Cedar text (inject corrupted text in test); 200 on valid rollback — verify new version_num is from_version + 1; verify policy in DB has restored text; verify reload triggered
- [ ] `cargo test --workspace && cargo clippy --workspace -- -D warnings`
