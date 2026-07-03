//! The poise gateway task + `BotData`.
//!
//! Implemented in Phase 3.

use std::sync::Arc;

use derive_more::Debug;
use error_stack::Report;
use jinn_domain::feat::discord::{DiscordConfig, DiscordThreadMap};
use jinn_domain::{Bridge, State};
use wherror::Error;

/// Error spawning the Discord gateway.
///
/// Returned when the bot token is missing or the poise framework cannot start.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SpawnError;

/// Shared context passed to every poise command and event handler.
///
/// Held behind `Arc<UserData>` by poise. Clones cheaply — every field is either
/// an `Arc`, a `Bridge` (which is `Clone`), or a channel handle.
#[derive(Debug, Clone)]
pub struct BotData {
    /// Read-only snapshot of the shared jinn `State` (sessions, frontend, etc.).
    /// The bot reads session history from here to extract the final reply.
    pub state: State,
    /// Clone of the actor bus — used to send `EnqueueUserMessage`,
    /// `SubmitSteeringMessage`, `SessionLoadRequested`, etc.
    pub bridge: Bridge,
    /// DAO over `sessions.db` for thread↔session mapping persistence.
    pub thread_map: DiscordThreadMap,
    /// The bot configuration from `jinn.toml` `[discord]`.
    pub config: Arc<DiscordConfig>,
}

/// Runs the Discord gateway: starts poise, registers slash commands, and
/// spawns the bridge-event drain loop. Blocks the calling task until the
/// gateway shuts down.
///
/// # Errors
///
/// Returns [`SpawnError`] if the bot token is missing/empty or the poise
/// framework fails to start.
#[expect(clippy::unused_async, reason = "poise gateway awaited in phase 3")]
pub async fn run(_data: BotData, token: String) -> Result<(), Report<SpawnError>> {
    if token.trim().is_empty() {
        tracing::error!("discord bot enabled but no token configured");
        return Err(Report::new(SpawnError));
    }
    // Full poise framework + drain loop wired up in Phase 3.
    tracing::warn!("jinn-discord gateway not yet implemented (phase 3 placeholder)");
    Ok(())
}
