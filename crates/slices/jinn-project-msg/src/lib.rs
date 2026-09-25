//! Canonical project contracts and shared picker vocabulary.

mod project_add_input_state;
mod project_entry;

pub use project_add_input_state::{ProjectAddInputState, project_add_slot};
pub use project_entry::{ProjectEntry, project_entries, render_project_row};
