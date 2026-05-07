//! Workflow panel state — selection, scroll, and detail toggle.
//!
//! Tracks which step the user has selected, the scroll offset for overflow,
//! and whether the detail view is active.

/// State for the workflow panel UI component.
#[derive(Debug, Clone, Default)]
pub struct WorkflowPanelState {
    /// Index of the currently selected step (in definition order).
    selected_index: usize,
    /// Vertical scroll offset for the step list.
    scroll_offset: u16,
    /// Whether the detail view is shown for the selected step.
    show_detail: bool,
}

impl WorkflowPanelState {
    /// Returns the index of the currently selected step.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Returns the current vertical scroll offset.
    #[must_use]
    pub fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    /// Returns whether the detail view is shown.
    #[must_use]
    pub fn show_detail(&self) -> bool {
        self.show_detail
    }

    /// Moves the selection to the next step.
    ///
    /// Clamps at the last step — does nothing if already at the end.
    pub fn select_next(&mut self, step_count: usize) {
        if step_count > 0 && self.selected_index < step_count - 1 {
            self.selected_index += 1;
        }
    }

    /// Moves the selection to the previous step.
    ///
    /// Clamps at the first step — does nothing if already at the beginning.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Moves the selection to the first step.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
    }

    /// Moves the selection to the last step.
    ///
    /// No-op if there are no steps.
    pub fn select_last(&mut self, step_count: usize) {
        if step_count > 0 {
            self.selected_index = step_count - 1;
        }
    }

    /// Toggles the detail view for the selected step.
    pub fn toggle_detail(&mut self) {
        self.show_detail = !self.show_detail;
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_next_increments_index() {
        // Given a panel state with 3 steps at index 0.
        let mut state = WorkflowPanelState::default();

        // When selecting next.
        state.select_next(3);

        // Then the selected index is 1.
        assert_eq!(state.selected_index(), 1);
    }

    #[test]
    fn select_next_clamps_at_last() {
        // Given a panel state with 3 steps at index 2.
        let mut state = WorkflowPanelState::default();
        state.select_next(3);
        state.select_next(3);
        assert_eq!(state.selected_index(), 2);

        // When selecting next.
        state.select_next(3);

        // Then the index stays at 2.
        assert_eq!(state.selected_index(), 2);
    }

    #[test]
    fn select_prev_decrements_index() {
        // Given a panel state with 3 steps at index 1.
        let mut state = WorkflowPanelState::default();
        state.select_next(3);

        // When selecting previous.
        state.select_prev();

        // Then the selected index is 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        // Given a panel state with 2 steps at index 0.
        let mut state = WorkflowPanelState::default();

        // When selecting previous.
        state.select_prev();

        // Then the index stays at 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn select_first_goes_to_index_zero() {
        // Given a panel state with 3 steps at index 2.
        let mut state = WorkflowPanelState::default();
        state.select_next(3);
        state.select_next(3);
        assert_eq!(state.selected_index(), 2);

        // When selecting first.
        state.select_first();

        // Then the selected index is 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[test]
    fn select_last_goes_to_last_index() {
        // Given a panel state with 3 steps at index 0.
        let mut state = WorkflowPanelState::default();

        // When selecting last.
        state.select_last(3);

        // Then the selected index is 2.
        assert_eq!(state.selected_index(), 2);
    }

    #[test]
    fn toggle_detail_flips_show_detail() {
        // Given a panel state with detail off.
        let mut state = WorkflowPanelState::default();
        assert!(!state.show_detail());

        // When toggling detail.
        state.toggle_detail();

        // Then detail is on.
        assert!(state.show_detail());

        // When toggling again.
        state.toggle_detail();

        // Then detail is off.
        assert!(!state.show_detail());
    }

    #[test]
    fn default_state_has_index_zero_no_detail() {
        // Given a default panel state.
        let state = WorkflowPanelState::default();

        // Then the defaults are correct.
        assert_eq!(state.selected_index(), 0);
        assert_eq!(state.scroll_offset(), 0);
        assert!(!state.show_detail());
    }
}
