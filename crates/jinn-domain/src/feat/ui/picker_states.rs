//! Picker state grouping and accessor trait.
//!
//! All picker-related state (picker widgets, snapshots, scroll offsets) is grouped
//! into [`PickerStates`]. The [`PickerExt`] extension trait provides accessor methods
//! on [`FrontendState`](super::FrontendState) so consumers are decoupled from the
//! internal storage layout.

use jinn_project_msg::ProjectEntry;

/// All picker state - grouped so the picker subsystem can evolve independently.
///
/// Each picker has its own selection state and optional companion fields
/// (snapshots, scroll offsets) used during the picker's open/close lifecycle.
#[derive(Debug, Default)]
pub struct PickerStates {
    /// Session picker state (items, filter text, selection index).

    /// Preview pane scroll offsets for spec-driven pickers, keyed by
    /// picker id.
    pub pickers_scrolls: jinn_picker::PickerScrolls,

    pub project_picker:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>,

    /// Measured results-area row count for the currently-active picker, as
    /// written by the TUI render pre-pass each frame. Used by the picker
    /// navigation intents to keep the cursor inside the visible window.
    ///
    /// Zero before the first render of a picker; the intent layer falls back
    /// to a sane default in that case.
    /// OWNER: TUI render pre-pass (writes) / IntentHandler (reads via
    /// `active_viewport`).
    pub picker_results_viewport: u16,
}

/// Extension trait providing typed access to picker state on [`FrontendState`](super::FrontendState).
///
/// Import this trait to access picker fields through methods instead of direct field access.
/// This decouples consumers from the internal storage layout of `FrontendState`.
pub trait PickerExt {
    /// Read-only access to the enabled MCP servers snapshot.
    /// Mutable access to the enabled MCP servers snapshot.
    /// Read-only access to the project picker state.
    fn project_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>;
    fn project_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>;

    /// The measured results-area row count, written once per frame by the
    /// render pre-pass and read by the page-up/page-down binds.
    fn picker_results_viewport(&self) -> u16;

    /// Updates the measured results-area row count. Called once per frame
    /// from the render pre-pass.
    fn set_picker_results_viewport(&mut self, val: u16);
}

impl PickerExt for super::frontend_state::FrontendState {
    fn project_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>> {
        &self.pickers.project_picker
    }

    fn project_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>> {
        &mut self.pickers.project_picker
    }

    fn picker_results_viewport(&self) -> u16 {
        self.pickers.picker_results_viewport
    }

    fn set_picker_results_viewport(&mut self, val: u16) {
        self.pickers.picker_results_viewport = val;
    }
}
