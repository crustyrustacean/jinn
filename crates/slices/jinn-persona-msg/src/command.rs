//! Persona slice commands that cross the actor bus.

use jinn_slices::BusMessage;
use serde::{Deserialize, Serialize};

/// Load entries for the persona picker.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load persona picker entries from the persona catalog.")]
pub struct LoadPersonaPickerEntries;

impl BusMessage for LoadPersonaPickerEntries {}
