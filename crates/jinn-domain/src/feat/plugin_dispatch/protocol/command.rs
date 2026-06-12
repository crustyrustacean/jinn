//! Plugin dispatch protocol — commands.
//!
//! Replaces the workflow command set. Plugins attach to a session by name;
//! the dispatcher loads them into a per-session Lua state and fires their
//! hooks at lifecycle events.

use serde::{Deserialize, Serialize};

use crate::common::bus::BusMessage;
use crate::protocol::SessionId;


/// Attach an attachable plugin to a session.
///
/// The plugin must exist under `plugins/attachable/`. The dispatcher will:
/// 1. Validate the plugin name.
/// 2. Call `services.plugins.create_session_registry([plugin_name])` to spin up
///    a per-session Lua state.
/// 3. Push `AttachedPlugin { name, enabled: true, run_state: Idle }` onto
///    `session.core.attached_plugins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachPlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// Detach a plugin from a session by name.
///
/// Calls `destroy_session_registry` for the matching registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachPlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// Toggle a plugin's `enabled` flag. No-op if not attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TogglePlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// Set the managed session ID on an attached plugin.
///
/// Called when a plugin creates a child session and wants the sidebar
/// to be able to navigate to it. The activate path reads this field
/// to determine which session to switch to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetManagedSession {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub managed_session_id: SessionId,
}

impl BusMessage for AttachPlugin {}
impl BusMessage for DetachPlugin {}
impl BusMessage for TogglePlugin {}
impl BusMessage for SetManagedSession {}
