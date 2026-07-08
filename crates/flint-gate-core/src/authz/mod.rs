//! Embedded Cedar authorization policy engine.
//!
//! This module wraps [`cedar_policy`] behind a small, lock-free-on-read facade:
//! a compiled [`CedarBundle`] (policies + optional schema + entities) is shared
//! via [`arc_swap::ArcSwap`] so the request hot path takes a snapshot without
//! locking. Policies are runtime-loadable from Postgres and hot-reloadable.
//!
//! ## Fail-closed contract
//!
//! This is authorization code. Every construction, parse, and evaluation error
//! resolves to [`Decision::Deny`]. The only path that yields [`Decision::Allow`]
//! is an explicit Cedar `Allow` response from a successfully built request
//! evaluated against a successfully compiled policy set. Ambiguity is denial.
//!
//! A hot-reload that fails to parse RETAINS the last-good bundle — a bad write
//! can never blank out the policy set and accidentally open (or, with a
//! default-deny policy set, hard-close) the gate.

mod bundle;
mod engine;
mod error;
mod sugar;
mod tool_authz;
mod validator;

pub use bundle::{CedarBundle, PolicyRecord};
pub use engine::{
    ApprovalContext, AuthzDecision, AuthzEngine, PrincipalKind, DEFAULT_ACTION,
    DEFAULT_APPROVAL_TTL_SECONDS,
};
pub use error::AuthzError;
pub use tool_authz::{
    authorize_tool_call, filter_list_tools_body, filter_list_tools_response, ToolAuditSink,
    ToolAuthzContext, ACTION_CALL_TOOL,
};
pub use sugar::{compile_agent_tool_policies, compile_and_validate, SUGAR_ID_PREFIX};
pub use validator::{policy_warnings, validate_policy, ALLOW_ALL_WARNING};

/// Re-export of Cedar's decision enum for callers that want to match on it.
pub use cedar_policy::Decision;
