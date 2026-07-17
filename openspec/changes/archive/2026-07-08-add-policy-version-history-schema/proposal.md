# add-policy-version-history-schema

## Summary

Add a `cedar_policy_versions` table to the Flint Gate schema and wire `Database::upsert_policy` to write a version row on every create-or-update, making policy changes reversible at the data layer.

## Motivation

Currently `Database::upsert_policy` performs a plain `INSERT … ON CONFLICT DO UPDATE` against `authz_policies`. Once a policy is overwritten the prior `policy_text` is gone — there is no history, no version numbering, and no way for an operator to recover from a bad policy change without external backups. The `cedar-policy-versioning-and-rollback` phase makes policy changes reversible; this change is the foundational data-layer prerequisite.

## Design

### Schema addition (idempotent `CREATE TABLE IF NOT EXISTS`)

Append to the existing `SCHEMA_SQL` constant in `crates/flint-gate-core/src/db/mod.rs`:

```sql
CREATE TABLE IF NOT EXISTS cedar_policy_versions (
    id            SERIAL PRIMARY KEY,
    policy_id     TEXT NOT NULL REFERENCES authz_policies(id) ON DELETE CASCADE,
    version_num   INT  NOT NULL,
    policy_text   TEXT NOT NULL,
    schema_json   JSONB,
    entities_json JSONB,
    written_by    TEXT,                          -- nullable: caller identity, deferred wiring
    written_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (policy_id, version_num)
);

CREATE INDEX IF NOT EXISTS cedar_policy_versions_policy_id_idx
    ON cedar_policy_versions (policy_id, version_num DESC);
```

`ON DELETE CASCADE` ensures version rows are automatically removed when a policy is hard-deleted from `authz_policies`.

### Version numbering

Application-level: `SELECT COALESCE(MAX(version_num), 0) + 1 FROM cedar_policy_versions WHERE policy_id = $1` inside a transaction with the upsert. This matches the `insert_nhi_audit` pattern already in the codebase (transaction-wrapped, application-controlled). A Postgres sequence-per-policy is unnecessary given low write volume.

### `Database::upsert_policy` extension

Wrap the existing plain upsert in a `begin()` transaction. After the main upsert, compute `next_version_num` and insert the version row. The method gains an optional `written_by: Option<&str>` parameter; all existing callers pass `None`.

```rust
pub async fn upsert_policy(
    &self,
    id: &str,
    policy_text: &str,
    schema_json: Option<&serde_json::Value>,
    entities_json: Option<&serde_json::Value>,
    enabled: bool,
    written_by: Option<&str>,          // NEW — nullable attribution
) -> Result<()>
```

### New DB methods

```rust
/// List version rows for a policy, newest first, with pagination.
pub async fn list_policy_versions(
    &self,
    policy_id: &str,
    offset: i64,
    limit: i64,
) -> Result<Vec<PolicyVersionRow>>

/// Fetch a single version row by (policy_id, version_num).
pub async fn get_policy_version(
    &self,
    policy_id: &str,
    version_num: i32,
) -> Result<Option<PolicyVersionRow>>
```

### New struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersionRow {
    pub id: i32,
    pub policy_id: String,
    pub version_num: i32,
    pub policy_text: String,
    pub schema_json: Option<serde_json::Value>,
    pub entities_json: Option<serde_json::Value>,
    pub written_by: Option<String>,
    pub written_at: DateTime<Utc>,
}
```

### Caller updates

All 4 call sites of `upsert_policy` in `admin/mod.rs` must pass `written_by: None` (or a future caller identity). No behavioral change — just the extra parameter.

## Tasks
- [ ] Add `cedar_policy_versions` DDL to `SCHEMA_SQL` (idempotent)
- [ ] Extend `Database::upsert_policy` signature with `written_by: Option<&str>` and wrap in a transaction that also inserts a version row
- [ ] Add `PolicyVersionRow` struct
- [ ] Add `Database::list_policy_versions` and `Database::get_policy_version` methods
- [ ] Update all 4 `upsert_policy` call sites in `admin/mod.rs` to pass `written_by: None`
- [ ] Add unit tests: version row inserted on first write, version_num increments on subsequent writes, `get_policy_version` returns correct row
- [ ] `cargo test --workspace && cargo clippy --workspace -- -D warnings`
