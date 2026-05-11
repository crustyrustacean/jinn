//! Snapshot of TUI signal flags, extracted from AppState before releasing the write lock.

/// Snapshot of [`nullslop_component::tui_signals::TuiSignals`] fields, copied
/// out of AppState before releasing the write lock.
#[derive(Debug)]
pub(super) struct TuiSignalsSnapshot {
    /// Whether to toggle the which-key overlay.
    pub(super) toggle_whichkey: bool,
    /// Whether an external editor was requested.
    pub(super) edit_requested: bool,
    /// Whether to toggle the pinned pane visibility.
    pub(super) pinned_pane_toggle: bool,
    /// Whether to open the pinned pane.
    pub(super) pinned_pane_open: bool,
    /// Whether to close the pinned pane.
    pub(super) pinned_pane_close: bool,
}

impl TuiSignalsSnapshot {
    /// Extracts TUI signal flags from the given app state.
    pub(super) fn from_state(state: &nullslop_component::AppState) -> Self {
        Self {
            toggle_whichkey: state.frontend.tui_signals.toggle_whichkey,
            edit_requested: state.frontend.tui_signals.edit_requested,
            pinned_pane_toggle: state.frontend.tui_signals.pinned_pane_toggle,
            pinned_pane_open: state.frontend.tui_signals.pinned_pane_open,
            pinned_pane_close: state.frontend.tui_signals.pinned_pane_close,
        }
    }
}
