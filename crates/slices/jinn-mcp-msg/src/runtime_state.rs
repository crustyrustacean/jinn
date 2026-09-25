//! Live MCP connection state, keyed by session.
//!
//! Durable server enablement remains part of the session snapshot. This value
//! stores only the independently live status and stderr projections produced by
//! MCP connection actors.

use std::collections::{BTreeMap, HashMap};

use jinn_core_types::SessionId;
use jinn_slices::SlotKey;

use crate::McpConnectionStatus;

/// Runtime-only MCP state for one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpSessionRuntimeState {
    /// Connection status by configured server name.
    pub status: BTreeMap<String, McpConnectionStatus>,
    /// Captured stderr tail by configured server name.
    pub stderr: BTreeMap<String, String>,
}

/// MCP runtime projections for all loaded sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpRuntimeState {
    sessions: HashMap<SessionId, McpSessionRuntimeState>,
}

impl McpRuntimeState {
    /// Sets one server's connection status for a session.
    pub fn set_status(
        &mut self,
        session_id: &SessionId,
        server: &str,
        status: McpConnectionStatus,
    ) {
        self.sessions
            .entry(session_id.clone())
            .or_default()
            .status
            .insert(server.to_owned(), status);
    }

    /// Returns one server's connection status.
    #[must_use]
    pub fn status(&self, session_id: &SessionId, server: &str) -> Option<McpConnectionStatus> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.status.get(server))
            .copied()
    }

    /// Returns a coherent copy of one session's status projection.
    #[must_use]
    pub fn statuses(&self, session_id: &SessionId) -> BTreeMap<String, McpConnectionStatus> {
        self.sessions
            .get(session_id)
            .map(|session| session.status.clone())
            .unwrap_or_default()
    }

    /// Replaces one server's captured stderr tail for a session.
    pub fn set_stderr(&mut self, session_id: &SessionId, server: &str, tail: String) {
        self.sessions
            .entry(session_id.clone())
            .or_default()
            .stderr
            .insert(server.to_owned(), tail);
    }

    /// Returns one server's captured stderr tail.
    #[must_use]
    pub fn stderr(&self, session_id: &SessionId, server: &str) -> Option<&str> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.stderr.get(server))
            .map(String::as_str)
    }

    /// Returns a coherent copy of one session's stderr projection.
    #[must_use]
    pub fn stderr_tails(&self, session_id: &SessionId) -> BTreeMap<String, String> {
        self.sessions
            .get(session_id)
            .map(|session| session.stderr.clone())
            .unwrap_or_default()
    }

    /// Removes all runtime data for one server in one session.
    pub fn clear_server(&mut self, session_id: &SessionId, server: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.status.remove(server);
            session.stderr.remove(server);
            if session.status.is_empty() && session.stderr.is_empty() {
                self.sessions.remove(session_id);
            }
        }
    }

    /// Removes all runtime data for one session.
    pub fn clear_session(&mut self, session_id: &SessionId) {
        self.sessions.remove(session_id);
    }
}

/// Stable cell slot for the live MCP runtime projection.
#[must_use]
pub fn mcp_runtime_slot() -> SlotKey {
    SlotKey::builtin("mcp", "runtime")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn set_status_is_visible_for_one_session_and_server() {
        // Given an empty MCP runtime projection.
        let mut runtime = McpRuntimeState::default();
        let session_id = SessionId::new();

        // When setting one server's status.
        runtime.set_status(&session_id, "weather", McpConnectionStatus::Running);

        // Then that status is readable for the same session and server.
        assert_eq!(
            runtime.status(&session_id, "weather"),
            Some(McpConnectionStatus::Running)
        );
    }

    #[rstest::rstest]
    fn clear_server_removes_status_and_stderr_for_that_server() {
        // Given a session with two MCP servers carrying runtime data.
        let session_id = SessionId::new();
        let mut runtime = McpRuntimeState::default();
        runtime.set_status(&session_id, "weather", McpConnectionStatus::Running);
        runtime.set_stderr(&session_id, "weather", "ready".to_owned());
        runtime.set_status(&session_id, "search", McpConnectionStatus::Starting);
        runtime.set_stderr(&session_id, "search", "starting".to_owned());

        // When clearing one server.
        runtime.clear_server(&session_id, "weather");

        // Then only that server's status and stderr are absent.
        assert_eq!(runtime.status(&session_id, "weather"), None);
        assert_eq!(runtime.stderr(&session_id, "weather"), None);
        assert!(runtime.statuses(&session_id).contains_key("search"));
        assert!(runtime.stderr_tails(&session_id).contains_key("search"));
    }

    #[rstest::rstest]
    fn clear_session_removes_all_runtime_data() {
        // Given runtime data for two sessions.
        let first = SessionId::new();
        let second = SessionId::new();
        let mut runtime = McpRuntimeState::default();
        runtime.set_status(&first, "weather", McpConnectionStatus::Running);
        runtime.set_status(&second, "weather", McpConnectionStatus::Dead);

        // When clearing the first session.
        runtime.clear_session(&first);

        // Then its projection is empty and the other session remains.
        assert!(runtime.statuses(&first).is_empty());
        assert_eq!(
            runtime.status(&second, "weather"),
            Some(McpConnectionStatus::Dead)
        );
    }
}
