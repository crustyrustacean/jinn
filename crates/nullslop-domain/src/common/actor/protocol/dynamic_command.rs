//! Dynamic command from a plugin or external source.

use crate::common::actor::command_msg::CommandMsg;
use serde::{Deserialize, Serialize};

/// A command carrying an arbitrary JSON payload, routed by runtime name.
///
/// Used by plugins to emit commands into the bus. If no actor subscribes
/// to the command's [`name`](Self::name), it is silently dropped.
///
/// The bus dispatches on the runtime `name` field (e.g. `"welcome::show"`)
/// rather than the static [`CommandMsg::NAME`] constant. This allows plugins
/// to define arbitrary routing keys without recompilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicCommand {
    /// Dot-namespaced identifier, e.g. `"welcome::show"`.
    pub name: String,
    /// Arbitrary JSON payload from the plugin.
    pub payload: serde_json::Value,
}

impl CommandMsg for DynamicCommand {
    const NAME: &'static str = "dynamic";
}
