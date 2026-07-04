//! `[discord]` configuration table for `jinn.toml`.
//!
//! When `enabled = true`, the TUI process spawns a Discord bot (via the
//! `jinn-discord` crate) that drives the same running jinn instance.
//! See `docs` (and `.plans/discord/plan.md`) for the full workflow.

use serde::{Deserialize, Serialize};

/// Discord bot configuration.
///
/// Serialized as the `[discord]` table in `jinn.toml`. All fields are optional
/// and default to a disabled bot — the bot only starts when `enabled = true`
/// AND a token is available (config field or `DISCORD_BOT_TOKEN` env var).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Master switch. `false` (default) = no bot, zero behavior change.
    #[serde(default)]
    pub enabled: bool,

    /// Bot token. If absent, the gateway reads the `DISCORD_BOT_TOKEN`
    /// environment variable at startup.
    #[serde(default)]
    pub bot_token: Option<String>,

    /// Name of the session lifecycle (from `[session_lifecycle]`) to run during
    /// `/new` setup. **Required** to use the bot — users who want no setup
    /// script declare a trivial lifecycle (e.g. `echo`). The lifecycle
    /// conventionally declares two params, `<branch>` and `<repo>`.
    #[serde(default)]
    pub lifecycle: Option<String>,

    /// Discord guild id (numeric, as a string) to scope slash-command
    /// registration. Slash commands registered globally take up to an hour to
    /// propagate; per-guild registration is instant and is the recommended dev
    /// setup. If absent, commands are registered globally.
    #[serde(default)]
    pub guild_id: Option<String>,
}
