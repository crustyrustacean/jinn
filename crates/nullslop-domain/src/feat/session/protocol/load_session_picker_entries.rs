//! Request to load session picker entries.
//!
//! Emitted when the session picker opens. The session actor receives this
//! command and responds by loading the list of saved sessions for display
//! in the picker UI.

use serde::{Deserialize, Serialize};

use crate::protocol::CommandMsg;

/// Request to load entries for the session picker.
///
/// This is a unit command — it carries no data. The session actor loads
/// all saved sessions and populates the picker state.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("session")]
pub struct LoadSessionPickerEntries;
