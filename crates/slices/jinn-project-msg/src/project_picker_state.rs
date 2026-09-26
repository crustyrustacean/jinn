//! The project picker's state and scope, shared by the slice and the kernel.
//!
//! The project picker lists curated project directories. Confirming one starts
//! a blank lifecycle at that directory; `<c-enter>` chains into the
//! session-lifecycle picker instead, and `<c-d>` removes the highlighted
//! project. All of that lives in the slice — this type is only the state the
//! rows need.

use crate::project_entry::ProjectEntry;
use jinn_slices::SliceScopeId;
use jinn_slices::SlotKey;

/// The cell slot holding [`ProjectPickerState`].
#[must_use]
pub fn project_picker_slot() -> SlotKey {
    SlotKey::builtin("project", "picker")
}

/// The scope the project picker's keys live in.
///
/// A dynamic slice scope, so the kernel holds no `Scope::PickerProject`
/// variant and the TUI layer never learns this picker exists.
#[must_use]
pub fn project_picker_scope() -> SliceScopeId {
    SliceScopeId::new("project", "picker")
}

/// Result rows assumed before the render pass has measured the real popup.
///
/// Matches the kernel's pre-measurement fallback, so the first keypress after
/// opening the picker pages the same way it always has.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// The project picker's complete state.
#[derive(Debug)]
pub struct ProjectPickerState {
    /// The selection/filter state backing the picker's rows.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>,
    /// Result rows the render pass measured, for half-page paging.
    pub results_viewport: usize,
}

impl Default for ProjectPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::default(),
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}
