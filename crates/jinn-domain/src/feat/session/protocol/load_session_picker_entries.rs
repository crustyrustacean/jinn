//! Load session picker entries command.

use serde::{Deserialize, Serialize};

use crate::BusMessage;

/// Load entries for the session picker.
///
/// The session persistence actor receives this, loads summaries from the session
/// store, and writes them into `AppState`.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load session picker entries from the session store.")]
pub struct LoadSessionPickerEntries;

impl BusMessage for LoadSessionPickerEntries {}
