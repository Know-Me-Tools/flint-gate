# merge-agent-tool-policies-into-engine

**Phase:** agent-governance-completeness-and-policy-authoring · **Goal:** G2 (build-002)
**Scope:** `crates/flint-gate-core/src/authz/engine.rs` (carry a sugar overlay +
concatenate on build/reload), `crates/flint-gate/src/main.rs` (remove the
refuse-start guard; seed the overlay), `crates/flint-gate-core/src/cache/mod.rs`
+ `crates/flint-gate-core/src/admin/mod.rs` (reload paths preserve the overlay),
docs.
**Depends on:** nothing (independent of G1; ordered after it).

## Why

Config `agent_tool_policies` sugar is enforced only in the config-only (no-DB)
deployment; with a database it **refuses to start** (`main.rs:306-315`), because
the DB-backed engine is built from `load_enabled_policies()` alone and
`reload_from_database` (`cache/mod.rs:372`, `admin/mod.rs:727`) is DB-only — any
sugar merged at startup would be dropped on the first policies reload. Operators
running the common DB deployment cannot use the sugar at all.

## What

1. **Carry the sugar overlay on the engine.** Store the validated
   `sugar_policies: Vec<PolicyRecord>` as an **immutable overlay** on `AuthzEngine`
   (set once at construction from config). Config stays the source of truth — the
   sugar is NOT written into `authz_policies` rows.
2. **Concatenate `DB records ++ sugar` in every build/reload path.**
   `from_database`, `reload_from_database`, and the admin-CRUD reload
   (`admin/mod.rs`) build the `CedarBundle` from the DB records **plus** the stored
   overlay, so the sugar is never dropped on reload. `CedarBundle::from_records`
   already merges any `&[PolicyRecord]` into one PolicySet — the only change is
   assembling the combined slice.
3. **Remove the refuse-start guard.** Delete the `db.is_some() &&
   !sugar_policies.is_empty()` bail (`main.rs:306-315`); the sugar now enforces
   alongside DB policies.

## Non-goals

- Custom cross-source precedence — Cedar `forbid-overrides-permit` is a formally
  verified guarantee, so a DB `forbid` beats a sugar `permit` (and vice-versa)
  under the merged PolicySet with no extra code.
- Config **hot-reload** of the sugar overlay — the overlay is fixed at startup;
  changing `agent_tool_policies` at runtime needs a restart (documented; a
  sugar-hot-reload is a separable follow-up).
- Changing the sugar compiler or its injection defense.

## Fail-safe requirement

The merge uses the existing parse-before-swap / lenient path (Cedar
`skip-on-error` + `default-deny`): a malformed row is dropped, never opens the
gate, and last-good is retained on a failed reload. Tested: sugar enforced with a
DB present; sugar survives a policies reload; conflict matrix — sugar-permit vs
DB-forbid → **deny**, DB-permit vs sugar-forbid → **deny**, two-permit → allow.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% coverage; merge + reload-persistence + precedence-matrix tests; the removed
guard's former refuse-start behavior replaced by an enforced-merge test.
