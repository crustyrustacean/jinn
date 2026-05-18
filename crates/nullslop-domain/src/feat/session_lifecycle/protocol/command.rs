//! Lifecycle command structs — async execution requests for setup/teardown.

use serde::{Deserialize, Serialize};

use crate::protocol::{CommandMsg, SessionId};

/// Request to run a lifecycle setup command asynchronously.
///
/// Sent by the `IntentHandler` when the user creates a session from a lifecycle
/// that has a `setup_command`. The session-persistence actor receives this,
/// runs the command via `run_lifecycle_command`, and updates the session state.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct RunSessionSetup {
    /// The session this setup is for.
    pub session_id: SessionId,
    /// The shell command to execute (already rendered with args).
    pub command: String,
    /// Positional arguments for the command.
    pub args: Vec<String>,
}

/// Request to run a lifecycle teardown command asynchronously.
///
/// Sent by the `IntentHandler` when the user closes a session whose lifecycle
/// has a `teardown_command`. The session-persistence actor receives this,
/// runs the command, and removes the session on success.
///
/// When `close_on_success` is `true`, the session is removed after successful
/// teardown. When `false`, the session is kept and a success info entry is
/// pushed instead (teardown-only mode).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct RunSessionTeardown {
    /// The session being torn down.
    pub session_id: SessionId,
    /// The shell command to execute (already rendered with args).
    pub command: String,
    /// Positional arguments (replayed from setup).
    pub args: Vec<String>,
    /// Whether to close the session on successful teardown.
    /// `true` for normal close (`x`), `false` for teardown-only (`t`).
    #[serde(default = "default_true")]
    pub close_on_success: bool,
}

fn default_true() -> bool {
    true
}
