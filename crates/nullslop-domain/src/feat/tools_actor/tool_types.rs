//! Tool calling types — execution context.
//!
//! [`ToolDefinition`], [`ToolCall`], and [`ToolResult`] are defined in the
//! `nullslop-provider` crate and re-exported here for convenience.
//! [`ToolContext`] remains in this module because it depends on domain types.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::common::actor::message_sink::MessageSink;
use crate::common::state::State;
use crate::protocol::SessionId;

// Re-export provider types.
pub use nullslop_provider::{ToolCall, ToolDefinition, ToolResult};

/// Context provided to every built-in tool at execution time.
///
/// Constructed by the tool orchestrator at dispatch time from session state.
/// Contains the session's CWD (for resolving relative paths), an optional
/// execution timeout, and an optional message sink for emitting streaming events.
#[derive(Clone)]
pub struct ToolContext {
    /// Working directory for resolving relative paths.
    pub cwd: PathBuf,
    /// Optional execution timeout.
    pub timeout: Option<Duration>,
    /// Shared application state (only available for tools that need it).
    pub state: Option<State>,
    /// Session ID (only available for tools that need it).
    pub session_id: Option<SessionId>,
    /// Application filesystem paths (for tools that need filesystem access).
    pub app_paths: crate::common::app_paths::AppPaths,
    /// Message sink for emitting streaming events.
    ///
    /// Only set for tools that need to emit incremental output events
    /// (e.g., bash streaming). When `None`, the tool runs silently
    /// and returns a single `ToolResult`.
    pub sink: Option<Arc<dyn MessageSink>>,
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolContext")
            .field("cwd", &self.cwd)
            .field("timeout", &self.timeout)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}
