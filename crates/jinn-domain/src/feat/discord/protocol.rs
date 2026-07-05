//! Wire protocol between the bridge actor and the poise gateway task.
//!
//! The [`crate::bridge_actor::DiscordBridgeActor`] subscribes to jinn's actor
//! bus and forwards the small set of events the Discord bot cares about onto a
//! bounded channel as [`BridgeEvent`]s. The poise gateway task drains that
//! channel and reacts.

use std::path::PathBuf;

/// The Discord session id (a string) tied to a jinn [`SessionId`](crate::SessionId).
///
/// Kept as a plain `String` because Discord ids arrive as strings from the
/// gateway and are stored as `TEXT` in the `discord_thread` table.
pub type ThreadId = String;

/// An event forwarded from the jinn bus to the poise gateway task.
///
/// Only the two transitions the bot reacts to are modeled:
/// - a turn finishing (session back to `Idle`) — the bot reads the final reply
/// - a setup finishing — the bot formats the setup result message
///
/// Everything else (streaming tokens, tool calls, intermediate entries) is
/// intentionally **not** forwarded — the bot only ever sends the final reply.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// A session's assistant turn finished (phase → `Idle`).
    ///
    /// The gateway reads the session's history from shared [`State`] to extract
    /// the final `Assistant` (or `Error`) entry.
    TurnFinished {
        /// The jinn session whose turn just ended.
        session_id: crate::SessionId,
    },
    /// A lifecycle setup completed (success or failure).
    ///
    /// Emitted by the session-persistence actor after running the setup script.
    /// The gateway formats a human-readable message from `cwd`/`error` and
    /// posts it to the thread bound to `session_id` via
    /// [`DiscordThreadMap::get_thread_by_session`].
    SetupCompleted {
        /// The session that was being set up.
        session_id: crate::SessionId,
        /// The resulting CWD on success, or default CWD on failure.
        cwd: PathBuf,
        /// Error message if setup failed.
        error: Option<String>,
    },
    /// A lifecycle teardown completed (success or failure).
    ///
    /// Emitted by the session-persistence actor after running the teardown script.
    /// The gateway formats a ✅/❌ message from `error` and posts it to the
    /// thread bound to `session_id`.
    TeardownFinished {
        /// The session that was being torn down.
        session_id: crate::SessionId,
        /// Error message if teardown failed.
        error: Option<String>,
    },
    /// A session was archived in persistent storage.
    ///
    /// Emitted by the session-persistence actor after marking the session
    /// archived in SQLite. The gateway posts a ✅ message to the bound thread.
    /// (`SessionArchived` itself carries no error field; archive never fails
    /// beyond DB write errors, which are logged at the actor.)
    Archived {
        /// The session that was archived.
        session_id: crate::SessionId,
    },
}
