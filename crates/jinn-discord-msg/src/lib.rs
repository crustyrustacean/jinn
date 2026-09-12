//! Discord crossing contracts.
//!
//! The EXPORT surface of the discord slice: every message that travels
//! between jinn's kameo bus and the poise gateway task (or the discord
//! status topic on the trouper fabric). The slice
//! (`jinn-discord-slice`) and the gateway frontend (`jinn-discord`)
//! both depend on this crate; the kernel never does.

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The trouper topic the discord status event crosses on.
#[must_use]
pub fn discord_topic() -> trouper::topics::Topic {
    trouper::topics::Topic::new("jinn.discord")
}

/// The Discord session id (a string) tied to a jinn [`SessionId`].
///
/// Kept as a plain `String` because Discord ids arrive as strings from
/// the gateway and are stored as `TEXT` in the `discord_thread` table.
pub type ThreadId = String;

impl jinn_slices::BusMessage for CreateThreadForSession {}
impl jinn_slices::BusMessage for DiscordThreadCreated {}
impl jinn_slices::BusMessage for DiscordThreadCreateFailed {}

/// An event forwarded from the jinn bus to the poise gateway task.
///
/// Only the transitions the bot reacts to are modeled:
/// - a turn finishing (session back to `Idle`) — the bot reads the final reply
/// - a lifecycle setup/teardown finishing — the bot posts the result
/// - a session being archived — the bot posts a ✅
///
/// Everything else (streaming tokens, tool calls, intermediate entries) is
/// intentionally **not** forwarded — the bot only ever sends the final reply.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// A session's assistant turn finished (phase → `Idle`).
    ///
    /// The gateway reads the session's history from shared [`State`] to extract
    /// the final `Assistant` (or `Error`) entry.
    ///
    /// [`State`]: jinn_domain::common::state::State
    TurnFinished {
        /// The jinn session whose turn just ended.
        session_id: SessionId,
    },
    /// A lifecycle setup completed (success or failure).
    ///
    /// Emitted by the session-persistence actor after running the setup script.
    /// The gateway formats a human-readable message from `cwd`/`error` and
    /// posts it to the thread bound to `session_id` via the thread map.
    SetupCompleted {
        /// The session that was being set up.
        session_id: SessionId,
        /// The resulting CWD on success, or default CWD on failure.
        cwd: PathBuf,
        /// Error message if setup failed.
        error: Option<String>,
    },
    /// A lifecycle teardown completed (success or failure).
    ///
    /// Emitted by the session-persistence actor after running the teardown
    /// script. The gateway formats a ✅/❌ message from `error` and posts it
    /// to the thread bound to `session_id`.
    TeardownFinished {
        /// The session that was being torn down.
        session_id: SessionId,
        /// Error message if teardown failed.
        error: Option<String>,
    },
    /// A session was archived in persistent storage.
    ///
    /// Emitted by the session-persistence actor after marking the session
    /// archived in SQLite. The gateway posts a ✅ message to the bound thread.
    /// (`Archived` itself carries no error field; archive never fails beyond
    /// DB write errors, which are logged at the actor.)
    Archived {
        /// The session that was archived.
        session_id: SessionId,
    },
}

// ── to-thread (jinn → Discord) ─────────────────────────────────────

/// Why a [`CreateThreadForSession`] request could not be fulfilled.
///
/// Carried by [`DiscordThreadCreateFailed`] so the feedback actor can render
/// a specific in-chat error message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreateThreadReason {
    /// The session is already bound to a Discord thread — re-`gdc` is rejected
    /// (never rebind, never orphan the existing thread).
    AlreadyBound,
    /// The configured `[discord] forum_channel` could not be resolved into a
    /// Discord forum channel.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
/// Published by the intent handler (on `gdc`); the core-bridge forward
/// route (staged at slice activation) carries it to the `jinn.session`
/// topic, where the slice's bridge subscriber turns it into a
/// [`GatewayRequest`] on the request channel. The gateway owns the
/// serenity `Http`, so Discord-mutating work is funneled through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateThreadForSession {
    /// The session to continue in Discord (bound to the new thread).
    pub session_id: SessionId,
    /// Title for the new Discord thread (the session's `title()`).
    pub title: String,
}

/// The gateway created the Discord thread and bound it to the session.
///
/// Published by the gateway after a successful thread creation; the
/// bridge subscriber appends a `ChatEntry::system` confirmation to the
/// session's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordThreadCreated {
    /// The session that now has a Discord thread.
    pub session_id: SessionId,
    /// The title the thread was created with (echoed for the confirmation msg).
    pub title: String,
}

/// The gateway could not create / bind the Discord thread.
///
/// Published by the gateway on a failed `gdc`; the bridge subscriber
/// appends a `ChatEntry::error` to the session's history. No thread is
/// created on `AlreadyBound` / `ForumChannel(_)`; a thread may exist on
/// Discord but be unbound on `MappingWriteFailed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordThreadCreateFailed {
    /// The session whose `gdc` failed.
    pub session_id: SessionId,
    /// Why it failed.
    pub reason: CreateThreadReason,
}

jinn_slices::crossing_schema!(CreateThreadForSession, "CreateThreadForSession",
trouper::schema::SchemaKind::Command,
description: "Lift a jinn session into a new Discord forum thread.",
fields: [
    "session_id" => trouper::schema::FieldTy::Uuid,
    "title" => trouper::schema::FieldTy::Str,
]);

jinn_slices::crossing_schema!(DiscordThreadCreated, "DiscordThreadCreated",
trouper::schema::SchemaKind::Event,
description: "A Discord forum thread was created and bound to a session.",
fields: [
    "session_id" => trouper::schema::FieldTy::Uuid,
    "title" => trouper::schema::FieldTy::Str,
]);

jinn_slices::crossing_schema!(DiscordThreadCreateFailed, "DiscordThreadCreateFailed",
trouper::schema::SchemaKind::Event,
description: "Discord thread creation failed; carries the reason.",
fields: [
    "session_id" => trouper::schema::FieldTy::Uuid,
    "reason" => trouper::schema::FieldTy::Json,
]);

/// A request from the jinn command path to the poise gateway task.
///
/// The gateway is the sole owner of the serenity `Http`, so any
/// Discord-mutating action is funneled through this enum over a kanal
/// channel (requests flow domain → gateway and carry *commands* the
/// gateway must *do*, as opposed to [`BridgeEvent`]s it reacts to).
#[derive(Debug, Clone)]
pub enum GatewayRequest {
    /// Create a forum thread under the configured `forum_channel`, named `title`,
    /// bound to `session_id`. Result reported via bus events
    /// ([`DiscordThreadCreated`] / [`DiscordThreadCreateFailed`]).
    CreateThreadForSession {
        /// The session to bind.
        session_id: SessionId,
        /// The new thread's title.
        title: String,
    },
}

// ── status (gateway → slice → dashboard) ───────────────────────────

/// Discord bot connection status, reported by the gateway task.
///
/// Crosses the trouper fabric as the payload of the `jinn.discord`
/// topic (schema [`DiscordStatusUpdate::schema_id`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscordStatusUpdate {
    /// The gateway is attempting to connect to Discord.
    Connecting,
    /// The gateway received its `ready` event — the bot is online.
    Connected,
    /// The websocket dropped mid-session.
    Disconnected,
    /// The gateway hit a fatal error (auth failure, unresolvable disconnect).
    Error {
        /// Human-readable reason (e.g. "401: invalid bot token").
        message: String,
    },
}

impl DiscordStatusUpdate {
    /// The dashboard entry name for the discord gateway. Discord facts
    /// live here, not in consumers — the dashboard folds this identity
    /// straight from the event.
    #[must_use]
    pub fn entry_name(&self) -> &'static str {
        "discord"
    }

    /// The dashboard entry description for the discord gateway.
    #[must_use]
    pub fn entry_description(&self) -> &'static str {
        "Discord gateway bot [Task]"
    }

    /// Renders the update into the dashboard `status_message` string.
    #[must_use]
    pub fn display_message(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::Error { .. } => "Error",
        }
    }

    /// Returns the full human-readable detail (for the `Error` variant).
    #[must_use]
    pub fn full_message(&self) -> String {
        match self {
            Self::Error { message } => format!("Error: {message}"),
            other => other.display_message().to_owned(),
        }
    }
}

jinn_slices::crossing_schema!(DiscordStatusUpdate, "DiscordStatusUpdate",
    trouper::schema::SchemaKind::Event,
    description: "Discord gateway connection status.",
    fields: []);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::DiscordStatusUpdate;

    #[rstest::rstest]
    #[test]
    fn status_update_roundtrips_through_json() {
        // Given a status update with a payload.
        let update = DiscordStatusUpdate::Error {
            message: "401: invalid bot token".to_owned(),
        };

        // When serializing and deserializing it.
        let json = serde_json::to_string(&update).unwrap();
        let round: DiscordStatusUpdate = serde_json::from_str(&json).unwrap();

        // Then the wire shape survives.
        assert!(
            matches!(round, DiscordStatusUpdate::Error { ref message } if message == "401: invalid bot token")
        );
    }
}
