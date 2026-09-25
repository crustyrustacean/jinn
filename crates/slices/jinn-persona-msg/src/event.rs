//! Persona slice events that cross the actor bus.

use crate::Persona;
use jinn_slices::BusMessage;
use serde::{Deserialize, Serialize};

/// Emitted when personas have been scanned and loaded from disk.
///
/// The context actor receives this event and stores the loaded personas
/// in `AppState`. If no active persona is set, the first one becomes default.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Persona directory scan completed.")]
pub struct PersonasLoaded {
    /// The loaded persona files.
    pub personas: Vec<Persona>,
    /// Error message if scanning failed, `None` on success.
    pub error: Option<String>,
}

impl BusMessage for PersonasLoaded {}
