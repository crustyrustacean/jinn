//! Commands emitted by plugins through `ctx.emit()`.
//!
//! The plugin system uses an untyped command boundary: plugins send a string
//! name and a JSON payload. The domain layer's command dispatcher
//! ([`crate::PluginSystem`] wiring) matches on the name and translates to
//! typed domain commands.

/// A command emitted by a plugin via `ctx.emit(name, data)`.
///
/// Carries the command name (string, matched by the dispatcher) and the
/// payload as a JSON value (arbitrary structured data from Lua).
#[derive(Debug, Clone)]
pub struct PluginCommand {
    /// Name of the plugin that emitted this command.
    pub plugin_name: String,
    /// The command name, e.g. `"push_chat_entry"`.
    pub name: String,
    /// The command payload as a JSON value.
    pub data: serde_json::Value,
}
