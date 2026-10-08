//! Errors from the graphs server, and the minigraf-code-preserving render
//! the tools return.

use wherror::Error;

/// Anything the graphs server refuses.
///
/// The wire message embeds minigraf's own stable code (`STG-026`, `PRS-001`,
/// …) verbatim, so a model reading the error can look the code up in the
/// minigraf error reference; remedies are appended where one exists.
#[derive(Debug, Error)]
#[error(debug)]
pub enum GraphsError {
    /// The graphs directory could not be created or read.
    #[error("graphs directory is unusable: {0}")]
    StoreDir(String),
    /// The graph name failed validation.
    #[error("invalid graph name: {0}")]
    BadName(String),
    /// The named graph does not exist.
    #[error("no graph named `{0}` — create it with create_graph first")]
    UnknownGraph(String),
    /// The graph already exists (create_graph only ever creates).
    #[error(
        "graph `{0}` already exists — open it by using transact/query; the existing file was left untouched"
    )]
    AlreadyExists(String),
    /// minigraf refused the operation (carries its rendered `[CODE] message`).
    #[error("minigraf error: {0}")]
    Minigraf(String),
}

impl GraphsError {
    /// minigraf's stable error code for a store failure, when one is present.
    ///
    /// `MinigrafError` renders as `[CODE] message`; this extracts the code so
    /// tests can assert on it without coupling to message wording.
    #[must_use]
    pub fn minigraf_code(rendered: &str) -> Option<&str> {
        let rest = rendered.strip_prefix('[')?;
        let end = rest.find(']')?;
        rest.get(..end)
    }
}

/// Builds an rmcp tool error carrying the rendered report.
///
/// MCP tool errors use `CallToolResult` error semantics — never jinn's
/// builtin `Error: ` text prefix, which is a builtin-tool wire convention.
pub fn tool_error(rendered: &str) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::internal_error(rendered.to_owned(), None)
}
