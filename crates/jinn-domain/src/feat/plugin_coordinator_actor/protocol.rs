//! Plugin status events — published by the plugin coordinator as each
//! plugin's runner child transitions through its lifecycle.
//!
//! No UI subscribes yet; the event exists so plugin health is observable
//! on the bus (and testable) from day one.

use serde::{Deserialize, Serialize};

/// Lifecycle state of one configured plugin's guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginPhase {
    /// The guest task spawned but the handshake has not completed.
    Starting,
    /// The handshake completed (`Hello` seen, `Welcome` sent).
    Running,
    /// The guest ended (crash, trap, or shutdown) or never came up.
    Dead,
    /// The guest lives but is flooding: the inbound channel filled and
    /// messages were dropped. Cleared back to `Running` when the channel
    /// drains.
    Unresponsive,
}

/// A lifecycle transition for one configured plugin.
///
/// Published by the plugin coordinator at every transition. Subscribers can
/// build a live view of every plugin process in the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatus {
    /// The configured plugin name (`jinn.toml` `[[plugin]].name`).
    pub name: String,
    /// The new phase.
    pub phase: PluginPhase,
}

impl crate::common::bus::BusMessage for PluginStatus {}
