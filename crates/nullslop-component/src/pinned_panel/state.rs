//! Pinned panel state — selection index within the pinned entries list.

/// State for the pinned context panel UI component.
#[derive(Debug, Clone, Default)]
pub struct PinnedPanelState {
    /// Index of the currently selected pinned entry.
    selection_index: usize,
}

impl PinnedPanelState {
    /// Returns the index of the currently selected pinned entry.
    #[must_use]
    pub fn selection_index(&self) -> usize {
        self.selection_index
    }

    /// Moves the selection to the next pinned entry.
    /// Clamps at `count - 1`. No-op if `count` is 0.
    pub fn select_next(&mut self, count: usize) {
        if count > 0 && self.selection_index < count - 1 {
            self.selection_index += 1;
        }
    }

    /// Moves the selection to the previous pinned entry.
    /// Clamps at 0.
    pub fn select_prev(&mut self) {
        if self.selection_index > 0 {
            self.selection_index -= 1;
        }
    }

    /// Resets the selection to index 0.
    pub fn reset_selection(&mut self) {
        self.selection_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_next_increments() {
        // Given a default panel state.
        let mut state = PinnedPanelState::default();

        // When selecting next with count 3.
        state.select_next(3);

        // Then the index is 1.
        assert_eq!(state.selection_index(), 1);
    }

    #[test]
    fn select_next_clamps_at_last() {
        // Given a panel state at index 2 with count 3.
        let mut state = PinnedPanelState::default();
        state.select_next(3);
        state.select_next(3);
        assert_eq!(state.selection_index(), 2);

        // When selecting next.
        state.select_next(3);

        // Then the index stays at 2.
        assert_eq!(state.selection_index(), 2);
    }

    #[test]
    fn select_next_is_noop_when_count_is_zero() {
        // Given a default panel state.
        let mut state = PinnedPanelState::default();

        // When selecting next with count 0.
        state.select_next(0);

        // Then the index stays at 0.
        assert_eq!(state.selection_index(), 0);
    }

    #[test]
    fn select_prev_decrements() {
        // Given a panel state at index 1.
        let mut state = PinnedPanelState::default();
        state.select_next(3);

        // When selecting previous.
        state.select_prev();

        // Then the index is 0.
        assert_eq!(state.selection_index(), 0);
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        // Given a default panel state at index 0.
        let mut state = PinnedPanelState::default();

        // When selecting previous.
        state.select_prev();

        // Then the index stays at 0.
        assert_eq!(state.selection_index(), 0);
    }

    #[test]
    fn reset_selection_goes_to_zero() {
        // Given a panel state at index 2.
        let mut state = PinnedPanelState::default();
        state.select_next(3);
        state.select_next(3);
        assert_eq!(state.selection_index(), 2);

        // When resetting selection.
        state.reset_selection();

        // Then the index is 0.
        assert_eq!(state.selection_index(), 0);
    }

    #[test]
    fn default_has_index_zero() {
        // Given a default panel state.
        let state = PinnedPanelState::default();

        // Then the index is 0.
        assert_eq!(state.selection_index(), 0);
    }
}
