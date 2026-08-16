//! Plugin status events — published by the plugin coordinator as each
//! plugin's runner child transitions through its lifecycle.
//!
//! No UI subscribes yet; the event exists so plugin health is observable
//! on the bus (and testable) from day one.

use serde::{Deserialize, Serialize};

/// Lifecycle state of one configured plugin's runner child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginPhase {
    /// The child process is spawned but the handshake has not completed.
    Starting,
    /// The handshake completed (`Hello` seen, `Welcome` sent).
    Running,
    /// The child exited (crash or shutdown) or never came up.
    Dead,
    /// The child lives but stopped reading: the bounded outbound queue
    /// overflowed and oldest events were dropped.
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
