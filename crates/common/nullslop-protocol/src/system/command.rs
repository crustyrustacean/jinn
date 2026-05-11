//! System commands.

use serde::{Deserialize, Serialize};

use crate::CommandMsg;
use crate::PickerKind;

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
