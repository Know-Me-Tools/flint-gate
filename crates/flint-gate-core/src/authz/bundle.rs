//! Compiled Cedar policy bundle and its construction from stored policy rows.
//!
//! A [`CedarBundle`] is the immutable, ready-to-evaluate artifact shared on the
//! hot path: a parsed [`PolicySet`], an optional [`Schema`], and an [`Entities`]
//! store. Bundles are built once (at startup or on reload) and then only read.

use std::str::FromStr;

use cedar_policy::{Entities, Policy, PolicyId, PolicySet, Schema};
use tracing::warn;

use super::error::AuthzError;

/// One authorization policy as loaded from the `authz_policies` table.
///
/// `schema_json`/`entities_json` are optional JSON blobs. When multiple enabled
/// rows exist, the FIRST non-null schema and the FIRST non-null entities blob
/// win (policies are otherwise merged). This keeps a single-schema model while
/// allowing many policy rows.
#[derive(Debug, Clone)]
pub struct PolicyRecord {
    /// Stable row id — also used as the Cedar `PolicyId` namespace prefix.
    pub id: String,
    /// Cedar policy source text (one or more `permit`/`forbid` statements).
    pub policy_text: String,
    /// Optional schema, either Cedar JSON schema or Cedar human syntax.
    pub schema_json: Option<serde_json::Value>,
    /// Optional entities store as Cedar JSON.
    pub entities_json: Option<serde_json::Value>,
}

/// A compiled, immutable Cedar authorization bundle.
///
/// Cloning is cheap-ish (Cedar's internal types are `Arc`-backed for the
/// policy AST); we build one and share it via `Arc` in [`super::AuthzEngine`].
pub struct CedarBundle {
    policies: PolicySet,
    schema: Option<Schema>,
    entities: Entities,
}

impl std::fmt::Debug for CedarBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CedarBundle")
            .field("policy_count", &self.policies.policies().count())
            .field("has_schema", &self.schema.is_some())
            .finish()
    }
}

impl CedarBundle {
    /// The compiled policy set.
    pub fn policies(&self) -> &PolicySet {
        &self.policies
    }

    /// The optional compiled schema.
    pub fn schema(&self) -> Option<&Schema> {
        self.schema.as_ref()
    }

    /// The entities store.
    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    /// An empty bundle: no policies, no schema, empty entities.
    ///
    /// With Cedar's default-deny semantics an empty policy set denies every
    /// request — the correct fail-closed startup state when no policies exist.
    pub fn empty() -> Self {
        Self {
            policies: PolicySet::new(),
            schema: None,
            entities: Entities::empty(),
        }
    }

    /// Build a bundle by merging a set of enabled policy rows.
    ///
    /// Each row's `policy_text` is parsed and its statements are added under a
    /// row-scoped id so two rows can't collide on an anonymous policy id. The
    /// first row carrying a schema/entities blob supplies the (single) schema
    /// and entities store. Any parse error fails the whole build — a bundle is
    /// all-or-nothing so a half-applied policy set never reaches the hot path.
    ///
    /// This STRICT variant is used by the write-time validation path (a write
    /// must be fully valid). The startup/reload path uses
    /// [`Self::from_records_lenient`] so one poisoned row cannot black-hole all
    /// authorization.
    pub fn from_records(records: &[PolicyRecord]) -> Result<Self, AuthzError> {
        let mut policy_set = PolicySet::new();
        let mut schema: Option<Schema> = None;
        let mut entities_value: Option<serde_json::Value> = None;

        for record in records {
            merge_record(record, &mut policy_set, &mut schema, &mut entities_value)?;
        }

        let entities = build_entities(entities_value, schema.as_ref())?;
        Ok(Self {
            policies: policy_set,
            schema,
            entities,
        })
    }

    /// Lenient build for startup/reload: skip-and-loudly-log any individual row
    /// that fails to compile (bad policy text, schema, or entities) and build
    /// from the SURVIVORS. One poisoned stored policy therefore degrades to
    /// "that policy is absent" rather than "all authorization is disabled".
    ///
    /// This never errors — the worst case is an empty (default-deny) bundle,
    /// which is the safe fail-closed floor. Each skipped row is logged at WARN
    /// with its id and the reason.
    pub fn from_records_lenient(records: &[PolicyRecord]) -> Self {
        let mut policy_set = PolicySet::new();
        let mut schema: Option<Schema> = None;
        let mut entities_value: Option<serde_json::Value> = None;

        for record in records {
            let mut candidate_set = policy_set.clone();
            let mut candidate_schema = schema.clone();
            let mut candidate_entities = entities_value.clone();
            match merge_record(
                record,
                &mut candidate_set,
                &mut candidate_schema,
                &mut candidate_entities,
            ) {
                Ok(()) => {
                    policy_set = candidate_set;
                    schema = candidate_schema;
                    entities_value = candidate_entities;
                }
                Err(e) => {
                    warn!(
                        policy_id = %record.id,
                        error = %e,
                        "skipping unparseable authz policy row — building from survivors"
                    );
                }
            }
        }

        // Entities may still fail to parse against the accumulated schema even
        // if each row was individually accepted above; fall back to empty
        // entities (fail-closed, never fail-open) rather than dropping the
        // whole bundle.
        let entities = match build_entities(entities_value, schema.as_ref()) {
            Ok(entities) => entities,
            Err(e) => {
                warn!(error = %e, "authz entities failed to compile — using empty entity store");
                Entities::empty()
            }
        };

        Self {
            policies: policy_set,
            schema,
            entities,
        }
    }
}

/// Compile a single record into the accumulating bundle pieces.
///
/// On success the row's policies are merged (re-id'd under the row id) and the
/// first-seen schema / entities blob is captured. On any failure NOTHING in the
/// caller's accumulators is left half-mutated: policies are staged in a local
/// set and only appended after the row fully parses, so a lenient caller can
/// discard exactly this row.
fn merge_record(
    record: &PolicyRecord,
    policy_set: &mut PolicySet,
    schema: &mut Option<Schema>,
    entities_value: &mut Option<serde_json::Value>,
) -> Result<(), AuthzError> {
    // Parse this row's policy text as its own set so we can re-id each
    // statement deterministically and merge into the combined set.
    let parsed = PolicySet::from_str(&record.policy_text)
        .map_err(|e| AuthzError::PolicyParse(format!("row `{}`: {e}", record.id)))?;

    // Stage the re-id'd policies locally; only commit to `policy_set` once the
    // whole row (policies + schema) has parsed.
    let mut staged: Vec<Policy> = Vec::new();
    for (idx, policy) in parsed.policies().enumerate() {
        let scoped_id = PolicyId::from_str(&format!("{}#{idx}", record.id))
            .map_err(|e| AuthzError::PolicyParse(format!("row `{}`: {e}", record.id)))?;
        staged.push(policy.new_id(scoped_id));
    }

    let parsed_schema = match (&schema, &record.schema_json) {
        (None, Some(schema_value)) => Some(parse_schema(schema_value)?),
        _ => None,
    };

    // Validate this row's entities blob (if any) BEFORE committing, against the
    // effective schema (accumulated schema, or this row's if it supplies one).
    // A bad entities blob therefore fails the whole row atomically — the lenient
    // loader can then skip exactly this poisoned row rather than dropping the
    // shared entity store or silently deferring the error to build time.
    let effective_schema = parsed_schema.as_ref().or(schema.as_ref());
    if let Some(entities_json) = &record.entities_json {
        Entities::from_json_value(entities_json.clone(), effective_schema)
            .map_err(|e| AuthzError::EntitiesParse(format!("row `{}`: {e}", record.id)))?;
    }

    // Commit — all parsing for this row succeeded.
    for policy in staged {
        policy_set
            .add(policy)
            .map_err(|e| AuthzError::PolicyParse(format!("row `{}`: {e}", record.id)))?;
    }
    if let Some(s) = parsed_schema {
        *schema = Some(s);
    }
    if entities_value.is_none() {
        if let Some(entities) = &record.entities_json {
            *entities_value = Some(entities.clone());
        }
    }
    Ok(())
}

/// Build the entity store from an optional JSON blob and optional schema.
fn build_entities(
    entities_value: Option<serde_json::Value>,
    schema: Option<&Schema>,
) -> Result<Entities, AuthzError> {
    match entities_value {
        Some(value) => Entities::from_json_value(value, schema)
            .map_err(|e| AuthzError::EntitiesParse(e.to_string())),
        None => Ok(Entities::empty()),
    }
}

/// Parse a schema value that may be either Cedar's JSON schema format (an
/// object) or Cedar human syntax carried as a JSON string.
fn parse_schema(value: &serde_json::Value) -> Result<Schema, AuthzError> {
    match value {
        // Human/Cedar syntax delivered as a JSON string.
        serde_json::Value::String(src) => {
            Schema::from_str(src).map_err(|e| AuthzError::SchemaParse(e.to_string()))
        }
        // Cedar JSON schema object.
        other => Schema::from_json_value(other.clone())
            .map_err(|e| AuthzError::SchemaParse(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, text: &str) -> PolicyRecord {
        PolicyRecord {
            id: id.to_string(),
            policy_text: text.to_string(),
            schema_json: None,
            entities_json: None,
        }
    }

    #[test]
    fn empty_bundle_has_no_policies() {
        let bundle = CedarBundle::empty();
        assert_eq!(bundle.policies().policies().count(), 0);
        assert!(bundle.schema().is_none());
    }

    #[test]
    fn from_records_merges_multiple_policy_rows() {
        let records = vec![
            record("p1", r#"permit(principal, action, resource);"#),
            record(
                "p2",
                r#"forbid(principal, action, resource) when { false };"#,
            ),
        ];
        let bundle = CedarBundle::from_records(&records).expect("should compile");
        assert_eq!(bundle.policies().policies().count(), 2);
    }

    #[test]
    fn from_records_rejects_malformed_policy() {
        let records = vec![record("bad", "this is not cedar policy {{{")];
        let err = CedarBundle::from_records(&records).unwrap_err();
        assert!(matches!(err, AuthzError::PolicyParse(_)));
    }

    #[test]
    fn from_records_parses_human_schema_string() {
        let mut rec = record("p1", r#"permit(principal, action, resource);"#);
        rec.schema_json = Some(serde_json::Value::String(
            "entity User; entity Resource; action \"invoke\" appliesTo { principal: [User], resource: [Resource] };".to_string(),
        ));
        let bundle = CedarBundle::from_records(&[rec]).expect("should compile with schema");
        assert!(bundle.schema().is_some());
    }

    #[test]
    fn from_records_rejects_bad_entities_json() {
        // A structurally invalid entities blob must fail the STRICT build so
        // write-time validation (which mirrors this) can reject it.
        let mut rec = record("p1", r#"permit(principal, action, resource);"#);
        rec.entities_json = Some(serde_json::json!({"not": "a valid entities array"}));
        let err = CedarBundle::from_records(&[rec]).unwrap_err();
        assert!(matches!(err, AuthzError::EntitiesParse(_)));
    }

    // ── H2: lenient loader skips poisoned rows, builds from survivors ────────

    #[test]
    fn lenient_loader_skips_bad_row_and_keeps_good_one() {
        let records = vec![
            record("good", r#"permit(principal, action, resource);"#),
            record("poisoned", "this is not cedar {{{"),
        ];
        // Strict would reject the whole set...
        assert!(CedarBundle::from_records(&records).is_err());
        // ...but lenient keeps the good policy and drops the bad one.
        let bundle = CedarBundle::from_records_lenient(&records);
        assert_eq!(
            bundle.policies().policies().count(),
            1,
            "only the good row's single policy should survive"
        );
    }

    #[test]
    fn lenient_loader_all_bad_yields_empty_default_deny() {
        let records = vec![
            record("b1", "garbage {{{"),
            record("b2", "more garbage )))"),
        ];
        let bundle = CedarBundle::from_records_lenient(&records);
        assert_eq!(
            bundle.policies().policies().count(),
            0,
            "all-bad input degrades to empty (default-deny), never fail-open"
        );
    }

    #[test]
    fn lenient_loader_drops_row_with_bad_entities() {
        let mut poisoned = record("poison", r#"permit(principal, action, resource);"#);
        poisoned.entities_json = Some(serde_json::json!({"bad": "entities"}));
        let records = vec![
            record("good", r#"permit(principal, action, resource);"#),
            poisoned,
        ];
        // The good row survives; the bad-entities row is skipped (its policy is
        // dropped and its entities blob never poisons the store).
        let bundle = CedarBundle::from_records_lenient(&records);
        assert_eq!(bundle.policies().policies().count(), 1);
    }
}
