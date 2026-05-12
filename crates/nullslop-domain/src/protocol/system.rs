//! System domain: application-level commands, events, and built-in actor commands.

use serde::{Deserialize, Serialize};

use crate::protocol::CommandMsg;
use crate::protocol::EventMsg;
use crate::protocol::Mode;
use crate::protocol::PickerKind;
use crate::protocol::key::KeyEvent;

// --- Commands ---

/// Load entries for the active picker from the actor system.
///
/// The provider actor receives this, calls the appropriate loader via `Services`,
/// and writes entries into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct LoadPickerEntries {
    /// Which picker kind to load entries for.
    pub kind: PickerKind,
}

// --- Events ---

/// A key was pressed down.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("system")]
pub struct KeyDown {
    /// The key event.
    pub key: KeyEvent,
}

/// A key was released.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("system")]
pub struct KeyUp {
    /// The key event.
    pub key: KeyEvent,
}

/// The application mode changed.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("system")]
pub struct ModeChanged {
    /// The previous mode.
    pub from: Mode,
    /// The new mode.
    pub to: Mode,
}
