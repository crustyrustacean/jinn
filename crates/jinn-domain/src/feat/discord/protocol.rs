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

// ── to-thread (reverse: jinn → Discord) ────────────────────────────

/// Why a `CreateThreadForSession` request could not be fulfilled.
///
/// Carried by [`DiscordThreadCreateFailed`] so the feedback actor can render
/// a specific in-chat error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateThreadReason {
    /// The session is already bound to a Discord thread — re-`gdc` is rejected
    /// (never rebind, never orphan the existing thread).
    AlreadyBound,
    /// The configured `[discord] forum_channel` could not be resolved into a
    /// Discord forum channel. Split into [`ForumChannelError::Missing`] (the
    /// field is unset/empty) and [`ForumChannelError::Invalid`] (it was set
    /// but doesn't parse as a numeric channel id / snowflake).
    ForumChannel(ForumChannelError),
    /// Discord rejected the thread creation (permissions, rate-limit, etc.).
    CreateFailed(String),
    /// The thread was created but the local thread↔session mapping write failed;
    /// the thread exists on Discord but is unbound.
    MappingWriteFailed,
}

/// Why the configured `forum_channel` couldn't be used to create a thread.
///
/// The gateway is the sole judge of whether `forum_channel` is usable, so
/// both cases surface here (never at the intent handler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForumChannelError {
    /// `[discord] forum_channel` is unset or empty.
    Missing,
    /// The field was set but isn't a valid Discord channel snowflake (it
    /// couldn't be parsed as a `u64`).
    Invalid {
        /// The raw, unparseable value exactly as configured.
        value: String,
    },
}

/// Bus command: lift the active jinn session into a new Discord forum thread.
///
/// Published by the intent handler (on `gdc`); the [`DiscordBridgeActor`] turns
/// it into a [`GatewayRequest`] on the request channel. The gateway owns the
/// serenity `Http`, so Discord-mutating work is funneled through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateThreadForSession {
    /// The session to continue in Discord (bound to the new thread).
    pub session_id: crate::SessionId,
    /// Title for the new Discord thread (the session's `title()`).
    pub title: String,
}

impl crate::common::bus::BusMessage for CreateThreadForSession {}

/// The gateway created the Discord thread and bound it to the session.
///
/// Subscribed to by the feedback actor, which appends a
/// `ChatEntry::system` confirmation to the session's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordThreadCreated {
    /// The session that now has a Discord thread.
    pub session_id: crate::SessionId,
    /// The title the thread was created with (echoed for the confirmation msg).
    pub title: String,
}

impl crate::common::bus::BusMessage for DiscordThreadCreated {}

/// The gateway could not create / bind the Discord thread.
///
/// Subscribed to by the feedback actor, which appends a `ChatEntry::error`
/// to the session's history. No thread is created on `AlreadyBound` /
/// `ForumChannel(_)`; a thread may exist on Discord but be unbound on
/// `MappingWriteFailed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordThreadCreateFailed {
    /// The session whose `gdc` failed.
    pub session_id: crate::SessionId,
    /// Why it failed.
    pub reason: CreateThreadReason,
}

impl crate::common::bus::BusMessage for DiscordThreadCreateFailed {}

/// A request from the jinn command path to the poise gateway task.
///
/// The gateway is the sole owner of the serenity `Http`, so any
/// Discord-mutating action is funneled through this enum over a second kanal
/// channel (the mirror image of the [`BridgeEvent`] channel — events flow
/// domain → gateway, requests flow domain → gateway too but carry *commands*
/// the gateway must *do* rather than *react to*).
#[derive(Debug, Clone)]
pub enum GatewayRequest {
    /// Create a forum thread under the configured `forum_channel`, named `title`,
    /// bound to `session_id`. Result reported via bus events
    /// ([`DiscordThreadCreated`] / [`DiscordThreadCreateFailed`]).
    CreateThreadForSession {
        /// The session to bind.
        session_id: crate::SessionId,
        /// The new thread's title.
        title: String,
    },
}
