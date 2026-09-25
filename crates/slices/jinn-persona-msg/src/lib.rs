//! Canonical contracts and shared vocabulary for the persona slice.

mod command;
mod event;
pub mod persona_entry;
pub mod persona_state;

pub use command::LoadPersonaPickerEntries;
pub use event::PersonasLoaded;
pub use persona_entry::{PersonaEntry, persona_row};
pub use persona_state::*;
