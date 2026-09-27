//! System domain: application-level events.

use serde::{Deserialize, Serialize};

use crate::common::bus::BusMessage;
use jinn_slices::{KeyEvent, Mode};

/// A key was pressed down.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyDown {
    /// The key event.
    pub key: KeyEvent,
}

/// A key was released.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyUp {
    /// The key event.
    pub key: KeyEvent,
}

/// The application mode changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeChanged {
    /// The previous mode.
    pub from: Mode,
    /// The new mode.
    pub to: Mode,
}
impl BusMessage for KeyDown {}
impl BusMessage for KeyUp {}

/// The active session changed.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "The active session changed.")]
pub struct ActiveSessionChanged {
    /// The new active session ID.
    pub session_id: jinn_core_types::SessionId,
}

impl BusMessage for ActiveSessionChanged {}
