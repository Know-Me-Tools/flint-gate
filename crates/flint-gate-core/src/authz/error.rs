//! Typed errors for the authorization engine.
//!
//! These are surfaced to the admin API (write-time validation) and to reload.
//! On the request hot path errors are NOT propagated — they are mapped to a
//! `Deny` decision (fail-closed) — but a typed error is still useful for logs
//! and for the admin CRUD path that must reject bad policy with a 400.

/// Errors produced while compiling, validating, or loading Cedar policy.
#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    /// A policy set failed to parse from its concatenated Cedar source.
    #[error("failed to parse policy set: {0}")]
    PolicyParse(String),

    /// A schema failed to parse (JSON or Cedar human syntax).
    #[error("failed to parse schema: {0}")]
    SchemaParse(String),

    /// The entities JSON failed to parse.
    #[error("failed to parse entities: {0}")]
    EntitiesParse(String),

    /// Write-time validation rejected the policy against the active schema.
    #[error("policy failed schema validation: {0}")]
    Validation(String),

    /// A Cedar request could not be constructed from the given principal /
    /// action / resource / context. On the hot path this maps to `Deny`.
    #[error("failed to build authorization request: {0}")]
    RequestBuild(String),

    /// The database layer returned an error while loading policies.
    #[error("failed to load policies from database: {0}")]
    Load(String),

    /// An `agent_tool_policies` sugar entry was malformed (empty/illegal agent id
    /// or tool name) and was rejected before compilation. Rejecting here keeps
    /// untrusted text out of the emitted Cedar source (injection-safe).
    #[error("invalid agent_tool_policies entry: {0}")]
    SugarCompile(String),
}
