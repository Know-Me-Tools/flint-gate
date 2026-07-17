# add-policy-history-endpoint

## Summary

Add `GET /policies/{id}/history` to the admin API, returning a paginated list of version rows for a policy ordered newest-first. Operators can use this to inspect the full edit trail before deciding whether to roll back.

## Motivation

With `cedar_policy_versions` in place (change `add-policy-version-history-schema`), the data exists but is not yet surfaced over HTTP. This change wires the endpoint that both the rollback endpoint and the admin UI depend on.

## Design

### Route

Register `.route("/policies/{id}/history", get(list_policy_history_handler))` in `admin/mod.rs` before the existing `"/policies/{id}"` wildcard route (or immediately after `/policies/simulate` — Axum literal-segment priority makes ordering safe, but keep it consistent with `validate` and `simulate` for readability).

### Query parameters

| Param | Default | Max | Notes |
|-------|---------|-----|-------|
| `offset` | 0 | — | integer |
| `limit` | 20 | 100 | clamped server-side |

### Handler

```rust
async fn list_policy_history_handler(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Query(params): Query<HistoryQueryParams>,
) -> impl IntoResponse
```

Returns 404 if the policy does not exist (check `db.get_policy(&id)` first). Returns 200 with:

```json
{
  "policy_id": "my-policy",
  "total_hint": null,
  "offset": 0,
  "limit": 20,
  "versions": [
    {
      "id": 7,
      "policy_id": "my-policy",
      "version_num": 3,
      "policy_text": "permit(...);",
      "schema_json": null,
      "entities_json": null,
      "written_by": null,
      "written_at": "2026-07-08T21:00:00Z"
    }
  ]
}
```

`total_hint` is intentionally `null` for this change (requires a `COUNT(*)` that adds query cost; deferred).

### Structs

```rust
#[derive(Deserialize)]
struct HistoryQueryParams {
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 { 20 }

#[derive(Serialize)]
struct PolicyHistoryResponse {
    policy_id: String,
    total_hint: Option<i64>,
    offset: i64,
    limit: i64,
    versions: Vec<PolicyVersionRow>,     // from db module
}
```

### DB call

Uses `db.list_policy_versions(&id, offset, clamp(limit, 1, 100))` from the previous change.

## Tasks
- [ ] Add `HistoryQueryParams` and `PolicyHistoryResponse` structs in `admin/mod.rs`
- [ ] Add `list_policy_history_handler` — 404 when policy not found, 200 with version list
- [ ] Register `.route("/policies/{id}/history", get(list_policy_history_handler))` before `/{id}`
- [ ] Add tests: 404 on unknown policy, empty list on new policy, versions returned in version_num DESC order, limit/offset pagination, limit clamped to 100
- [ ] `cargo test --workspace && cargo clippy --workspace -- -D warnings`
