//! Lifecycle command structs - async execution requests for setup/teardown.

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
    /// The original lifecycle command, used to dispatch builtin vs shell.
    /// `None` for backward compatibility with existing callers.
    #[serde(default)]
    pub lifecycle_command: Option<crate::feat::session_lifecycle::builtin::LifecycleCommand>,
}

/// Request to run a lifecycle teardown command asynchronously.
///
/// Sent by the sidebar teardown handler when the user triggers teardown-only
/// mode (`t` key). The session-persistence actor receives this, runs the command,
/// advances `lifecycle_script_state` to `TeardownRan`, persists, and emits
/// `SessionTeardownFinished`. The session is NOT removed from memory.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct RunSessionTeardown {
    /// The session being torn down.
    pub session_id: SessionId,
    /// The shell command to execute (already rendered with args).
    pub command: String,
    /// Positional arguments (replayed from setup).
    pub args: Vec<String>,
}

/// Request to persist a session to SQLite immediately.
///
/// Emitted by the `IntentHandler` alongside `RunSessionSetup` so the session
/// is saved before the setup command even begins executing. This ensures the
/// session's lifecycle metadata (name, args) survives an app crash during setup.
/// Also used by other flows (teardown, archive) that need to persist state changes.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct PersistSession {
    /// The session to persist.
    pub session_id: SessionId,
}

/// Result of an async teardown shell command, sent back to the session actor.
///
/// Emitted by the tokio task spawned during `handle_run_session_teardown` or
/// `handle_close_session`. The session actor processes this to advance lifecycle
/// state, emit events, and (if `close_after` is true) archive/remove the session.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct FinishSessionTeardown {
    /// The session that was being torn down.
    pub session_id: SessionId,
    /// Whether this teardown was triggered by a close operation.
    /// When true, the handler archives/removes the session after advancing lifecycle.
    pub close_after: bool,
    /// Error message if teardown failed.
    pub error: Option<String>,
}
