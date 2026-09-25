//! Project-add popup vocabulary — its edit state and the cell slot identity.

use jinn_slices::{LineInput, SlotKey};

/// State for the project-add input popup — editing a directory path.
///
/// The editable text and cursor live in [`LineInput`] under
/// [`ProjectAddInputState::text`].
#[derive(Debug, Clone, Default)]
pub struct ProjectAddInputState {
    /// The editable text and cursor.
    pub text: LineInput,
}

/// The cell slot holding the project-add popup's single edit state.
#[must_use]
pub fn project_add_slot() -> SlotKey {
    SlotKey::builtin("project", "project_add")
}
