//! The saved-attendants picker's wire contracts: its cell slot, dynamic
//! scope, and state.
//!
//! The picker browses `[[attendant.entry]]` — the attendants the user has
//! saved from the properties popup — and confirms one to create it on the
//! active session. Like every picker, the payload lives in the `-msg` crate
//! and the slice owns the rows, rendering, and the creation itself.
//!
//! The payload is [`SavedAttendantSummary`], a flat view of one entry, not
//! the config type itself: the config crate depends on *this* crate for its
//! behavior/trigger vocabulary, so a cell holding the config entry would
//! close the loop into a cycle. The summary also keeps the cell renderable
//! without a `jinn.toml` read on every frame.

use jinn_slices::SlotKey;

/// The `attendant/saved-picker` slot: the saved-attendants picker's state.
#[must_use]
pub fn attendant_saved_picker_slot() -> SlotKey {
    SlotKey::builtin("attendant", "saved-picker")
}

/// The saved-attendants picker's dynamic scope.
#[must_use]
pub fn attendant_saved_picker_scope() -> jinn_slices::slice_scope::SliceScopeId {
    jinn_slices::slice_scope::SliceScopeId::new("attendant", "saved-picker")
}

/// Result rows assumed before the render pass has measured the real popup.
///
/// Matches the kernel's pre-measurement fallback, so the first keypress
/// after opening the picker pages the same way it always has.
pub const RESULTS_VIEWPORT_FALLBACK: usize = 20;

/// One row in the saved-attendants picker.
///
/// The name is the whole entry: it is what the row shows, and it is the
/// key the created attendant is looked up by. The run configuration the
/// entry carries is not summarized here — the picker creates an attendant,
/// it does not describe one, and a row that previewed a configuration the
/// user cannot change from this popup is a second thing to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedAttendantSummary {
    /// The entry's name — also the title the created attendant is given.
    pub name: String,
}

/// State for the saved-attendants picker.
#[derive(Debug)]
pub struct AttendantSavedPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<SavedAttendantSummary>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}

impl Default for AttendantSavedPickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::default(),
            results_viewport: RESULTS_VIEWPORT_FALLBACK,
        }
    }
}
