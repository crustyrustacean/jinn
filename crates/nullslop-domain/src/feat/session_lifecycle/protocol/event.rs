//! Lifecycle event structs — completion callbacks for setup/teardown.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::protocol::{EventMsg, SessionId};

/// Setup command completed (success or failure).
///
/// Emitted by the session-persistence actor after running a lifecycle setup command.
/// On success, `cwd` is the directory reported by the command. On failure, `cwd`
/// is the default CWD and `error` contains the failure details.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session_lifecycle")]
pub struct SessionSetupCompleted {
    /// The session that was being set up.
    pub session_id: SessionId,
    /// The resulting CWD on success, or default CWD on failure.
    pub cwd: PathBuf,
    /// Error message if setup failed.
    pub error: Option<String>,
}

/// Teardown command completed (success or failure).
///
/// Emitted by the session-persistence actor after running a lifecycle teardown command.
/// On success, the session has already been removed from the sessions map.
/// On failure, the session is still open and `error` describes the problem.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("session_lifecycle")]
pub struct SessionTeardownCompleted {
    /// The session that was being torn down.
    pub session_id: SessionId,
    /// Error message if teardown failed.
    pub error: Option<String>,
}
