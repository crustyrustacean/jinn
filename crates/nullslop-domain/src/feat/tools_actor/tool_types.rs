//! Tool calling types — execution context.
//!
//! [`ToolDefinition`], [`ToolCall`], and [`ToolResult`] are defined in the
//! `nullslop-provider` crate and re-exported here for convenience.
//! [`ToolContext`] remains in this module because it depends on domain types.

use std::path::PathBuf;
use std::time::Duration;

// Re-export provider types.
pub use nullslop_provider::{ToolCall, ToolDefinition, ToolResult};

/// Context provided to every built-in tool at execution time.
///
/// Constructed by the tool orchestrator at dispatch time from session state.
/// Contains the session's CWD (for resolving relative paths) and an optional
/// execution timeout.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Working directory for resolving relative paths.
    pub cwd: PathBuf,
    /// Optional execution timeout.
    pub timeout: Option<Duration>,
    /// Shared application state (only available for tools that need it).
    pub state: Option<crate::common::state::State>,
    /// Session ID (only available for tools that need it).
    pub session_id: Option<crate::protocol::SessionId>,
    /// Application filesystem paths (for tools that need filesystem access).
    pub app_paths: crate::common::app_paths::AppPaths,
}
