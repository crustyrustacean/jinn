//! Events emitted by the plugin actor.

use serde::{Deserialize, Serialize};

use crate::protocol::EventMsg;

/// An event from a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("plugin_actor")]
pub struct PluginEvent {
    /// The plugin that emitted this event.
    pub plugin_id: String,
    /// The event name.
    pub event_name: String,
}
