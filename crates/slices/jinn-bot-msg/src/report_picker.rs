//! The bot report-history picker's wire contracts: its cell slot,
//! dynamic scope, and state.
//!
//! The picker is read-only — a browser over one bot's full report
//! log — so its state is the selection widget's state and nothing else.
//! Like every picker, the payload lives in the `-msg` crate, never the
//! slice implementation.

use jinn_slices::SlotKey;

/// The `bot/report-picker` slot: the report-history picker's state.
#[must_use]
pub fn bot_report_picker_slot() -> SlotKey {
    SlotKey::builtin("bot", "report-picker")
}

/// The report-history picker's dynamic scope.
#[must_use]
pub fn bot_report_picker_scope() -> jinn_slices::slice_scope::SliceScopeId {
    jinn_slices::slice_scope::SliceScopeId::new("bot", "report-picker")
}

/// State for the bot report-history picker.
#[derive(Debug, Default)]
pub struct BotReportPickerState {
    /// The rows, the filter text, and the highlight.
    pub selection:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<crate::BotReport>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
}
