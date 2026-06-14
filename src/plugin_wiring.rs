//! Plugin wiring — command dispatcher.
//!
//! Maps plugin command names (strings) to typed domain messages by delegating
//! to [`jinn_domain::common::plugin_bridge::dispatch_verb`]. Each verb's
//! Lua→domain translation lives on the domain message itself via the
//! `TryFromLua` trait in `plugin_bridge`.
//!
//! This module is a thin caller: it builds a [`CmdCtx`], asks the domain to
//! dispatch, and forwards the resulting [`BridgeClosure`] to the [`Bridge`]
//! (the same live channel the rest of the app uses).

use std::sync::Arc;

use jinn_domain::common::bridge::Bridge;
use jinn_domain::common::plugin_bridge::{CmdCtx, dispatch_verb};
use jinn_domain::feat::plugin_system::PluginCommand;

/// Dispatch a plugin command to the appropriate domain action.
///
/// Delegates verb matching and Lua→message translation to [`dispatch_verb`].
/// On success the returned closure is forwarded to the actor system via
/// [`Bridge::send`]. Unknown verbs and translation failures are logged and
/// dropped.
pub fn handle_plugin_command(cmd: PluginCommand, bridge: &Bridge) {
    tracing::debug!(
        plugin = cmd.plugin_name,
        verb = cmd.name,
        "plugin command dispatched"
    );

    let ctx = CmdCtx {
        plugin_name: cmd.plugin_name.clone(),
        verb: cmd.name.clone(),
    };

    match dispatch_verb(&cmd.name, ctx, cmd.data) {
        Some(closure) => {
            let _ = bridge.send(closure);
        }
        None => tracing::warn!(
            plugin = cmd.plugin_name,
            verb = cmd.name,
            "unknown plugin verb"
        ),
    }
}

/// Build a command dispatcher closure for the plugin system.
///
/// The returned closure captures the [`Bridge`] and routes plugin commands
/// through it to the kameo bus.
pub fn build_command_dispatcher(bridge: Bridge) -> Arc<dyn Fn(PluginCommand) + Send + Sync> {
    Arc::new(move |cmd: PluginCommand| {
        handle_plugin_command(cmd, &bridge);
    })
}

// ─── Request handler (for ctx.request from Lua) ────────────────────

/// Handle a request from an async hook's `ctx.request(name, data)` call.
///
/// Returns a result envelope: `{ ok: true, value }` on success, or
/// `{ ok: false, error }` on any failure.
// FIXME: plugin migration — re-enable once DomainNodeContext is restored
pub async fn handle_plugin_request(
    name: &str,
    _data: &serde_json::Value,
    _domain_ctx: &jinn_domain::feat::plugin_dispatch::DomainNodeContext,
) -> serde_json::Value {
    tracing::warn!(name, "plugin request handler not yet re-enabled");
    request_err(format_args!("not yet re-enabled: {name}"))
}

fn request_err(error: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": error.to_string() })
}
