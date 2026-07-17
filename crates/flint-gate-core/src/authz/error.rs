//! Typed errors for the authorization engine.
//!
//! These are surfaced to the admin API (write-time validation) and to reload.
//! On the request hot path errors are NOT propagated — they are mapped to a
//! `Deny` decision (fail-closed) — but a typed error is still useful for logs
//! and for the admin CRUD path that must reject bad policy with a 400.

/// A single Cedar parse or validation error with source location.
///
/// `line` and `column` are 1-based. `length` is the number of characters
/// (bytes) the error spans starting at `column`. All three are 0 when the
/// Cedar SDK does not provide a source location (e.g. validation errors
/// referencing an unnamed position).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PolicyParseError {
    pub line: usize,
    pub column: usize,
    pub length: usize,
    pub message: String,
}

impl PolicyParseError {
    /// Build from a Cedar [`ParseError`] (implements [`miette::Diagnostic`])
    /// and the original source text. Extracts the primary labeled span's byte
    /// offset and converts it to a 1-based line/column by scanning the source.
    /// Falls back to `(0, 0, 0)` when no span is available.
    pub fn from_parse_error(src: &str, err: &cedar_policy::ParseError) -> Self {
        use miette::Diagnostic;
        let (line, column, length) = match err.labels() {
            Some(mut labels) => match labels.next() {
                Some(span) => {
                    let offset = span.offset();
                    let len = span.len();
                    let (l, c) = byte_offset_to_line_col(src, offset);
                    (l, c, len)
                }
                None => (0, 0, 0),
            },
            None => (0, 0, 0),
        };
        Self {
            line,
            column,
            length,
            message: err.to_string(),
        }
    }

    /// Build from a Cedar [`ValidationError`] (implements [`miette::Diagnostic`])
    /// and the original source text. Same span extraction as [`from_parse_error`].
    pub fn from_validation_error(src: &str, err: &cedar_policy::ValidationError) -> Self {
        use miette::Diagnostic;
        let (line, column, length) = match err.labels() {
            Some(mut labels) => match labels.next() {
                Some(span) => {
                    let offset = span.offset();
                    let len = span.len();
                    let (l, c) = byte_offset_to_line_col(src, offset);
                    (l, c, len)
                }
                None => (0, 0, 0),
            },
            None => (0, 0, 0),
        };
        Self {
            line,
            column,
            length,
            message: err.to_string(),
        }
    }

    /// Build a simple error with no source location (for context messages that
    /// wrap a row id prefix or other non-parse failures).
    pub fn without_location(msg: String) -> Self {
        Self {
            line: 0,
            column: 0,
            length: 0,
            message: msg,
        }
    }
}

impl std::fmt::Display for PolicyParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "line {}, col {}: {}", self.line, self.column, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

/// Convert a byte offset into a 1-based (line, column) pair by scanning the
/// source string. Both values are 1-based. Returns `(1, 1)` for offset 0 or
/// any offset beyond the string's length.
fn byte_offset_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Errors produced while compiling, validating, or loading Cedar policy.
#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    /// A policy set failed to parse from its concatenated Cedar source.
    ///
    /// Carries structured per-error details with source locations.
    #[error("failed to parse policy set: {}", format_parse_errors(.0))]
    PolicyParse(Vec<PolicyParseError>),

    /// A schema failed to parse (JSON or Cedar human syntax).
    #[error("failed to parse schema: {0}")]
    SchemaParse(String),

    /// The entities JSON failed to parse.
    #[error("failed to parse entities: {0}")]
    EntitiesParse(String),

    /// Write-time validation rejected the policy against the active schema.
    ///
    /// Carries structured per-error details with source locations.
    #[error("policy failed schema validation: {}", format_parse_errors(.0))]
    Validation(Vec<PolicyParseError>),

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

fn format_parse_errors(errors: &[PolicyParseError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
