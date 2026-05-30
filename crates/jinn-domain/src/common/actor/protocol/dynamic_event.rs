//! Dynamic event from a plugin or external source.

use crate::common::actor::event_msg::EventMsg;
use serde::{Deserialize, Serialize};

/// An event carrying an arbitrary JSON payload, broadcast by runtime name.
///
/// Used by plugins to publish events. If no actor subscribes
/// to the event's [`name`](Self::name), it is silently dropped.
///
/// The bus broadcasts on the runtime `name` field (e.g. `"app::started"`)
/// rather than the static [`EventMsg::TYPE_NAME`] constant. This allows plugins
/// to define arbitrary event names without recompilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicEvent {
    /// Dot-namespaced identifier, e.g. `"app::started"`.
    pub name: String,
    /// Arbitrary JSON payload from the plugin.
    pub payload: serde_json::Value,
}

impl EventMsg for DynamicEvent {
    const TYPE_NAME: &'static str = "dynamic";
}
