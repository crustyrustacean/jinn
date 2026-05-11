//! Entry selection methods for [`ChatSessionState`](super::ChatSessionState).

use nullslop_protocol::{ChatEntry, ChatEntryId};

use super::ChatSessionState;

impl ChatSessionState {
    /// Select the next entry (moving toward newer messages).
    ///
    /// If nothing is selected, selects the first entry.
    /// Clamps to the last entry index.
    /// No-op if history is empty.
    pub fn select_next_entry(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let max = self.core.history.len() - 1;
        self.ui.selected_entry_index = Some(
            self.ui
                .selected_entry_index
                .map_or(0, |i| i.saturating_add(1).min(max)),
        );
    }

    /// Select the previous entry (moving toward older messages).
    ///
    /// If nothing is selected, selects the last entry.
    /// Clamps to 0.
    /// No-op if history is empty.
    pub fn select_prev_entry(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        self.ui.selected_entry_index = Some(
            self.ui
                .selected_entry_index
                .map_or(self.core.history.len() - 1, |i| i.saturating_sub(1)),
        );
    }

    /// Clear the entry selection.
    pub fn clear_selection(&mut self) {
        self.ui.selected_entry_index = None;
    }

    /// The index of the currently selected entry, if any.
    pub fn selected_entry_index(&self) -> Option<usize> {
        self.ui.selected_entry_index
    }

    /// The currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&ChatEntry> {
        let i = self.ui.selected_entry_index?;
        self.core.history.get(i)
    }

    /// The ID of the currently selected entry, if any.
    pub fn selected_entry_id(&self) -> Option<&ChatEntryId> {
        self.selected_entry().map(|e| &e.id)
    }
}
