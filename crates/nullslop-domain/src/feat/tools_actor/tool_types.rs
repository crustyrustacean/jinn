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
    /// Shell binary path (captured at startup from $SHELL).
    pub shell: String,
    /// Maximum lines for tool output truncation. `None` uses built-in default.
    pub max_output_lines: Option<usize>,
    /// Maximum bytes for tool output truncation. `None` uses built-in default.
    pub max_output_bytes: Option<usize>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[rstest::rstest]
    fn tool_context_debug_contains_cwd_and_timeout() {
        // Given a ToolContext with known values.
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp/test"),
            timeout: Some(std::time::Duration::from_secs(30)),
            state: None,
            session_id: Some(crate::protocol::SessionId::new()),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };

        // When debugging.
        let debug_str = format!("{ctx:?}");

        // Then the output contains cwd, timeout, and session_id.
        assert!(debug_str.contains("/tmp/test"), "debug should contain cwd");
        assert!(debug_str.contains("timeout"), "debug should contain timeout");
        assert!(debug_str.contains("session_id"), "debug should contain session_id");
        assert!(debug_str.contains("ToolContext"), "debug should contain struct name");
    }
}
