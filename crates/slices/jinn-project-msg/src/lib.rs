//! Canonical project contracts and shared picker vocabulary.

mod project_add_input_state;
mod project_entry;
mod project_picker_state;

pub use project_add_input_state::{ProjectAddInputState, project_add_slot};
pub use project_entry::{ProjectEntry, project_entries, render_project_row};
pub use project_picker_state::{
    ProjectPickerState, RESULTS_VIEWPORT_FALLBACK, project_picker_scope, project_picker_slot,
};
