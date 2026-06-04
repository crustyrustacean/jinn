//! Plugin wiring — command dispatcher.
//!
//! This file is the only place that maps plugin command names (strings)
//! to typed domain Commands. Plugins call `ctx.emit("command_name", { ... })`
//! and the dispatcher matches on the name.
//!
//! Currently a stub — will be fully implemented in Phase 6.

use jinn_plugin::PluginCommand;

/// Dispatch a plugin command to the appropriate domain action.
///
/// Stub implementation — logs and drops commands.
pub fn handle_plugin_command(cmd: PluginCommand) {
    tracing::debug!(name = cmd.name, "plugin command received (stub)");
}
