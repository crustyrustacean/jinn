//! Plugin dispatch events.

use serde::{Deserialize, Serialize};

use crate::protocol::{SessionId};

/// A plugin was attached to a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAttached {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// A plugin was detached from a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDetached {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// A plugin was toggled on/off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToggled {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub enabled: bool,
}

impl crate::common::bus::BusMessage for PluginAttached {}
impl crate::common::bus::BusMessage for PluginDetached {}
impl crate::common::bus::BusMessage for PluginToggled {}
