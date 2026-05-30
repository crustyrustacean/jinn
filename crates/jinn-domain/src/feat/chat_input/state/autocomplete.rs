//! Autocomplete state for prompt-template and slash-command completion.
//!
//! Tracks the active autocomplete session: trigger kind, trigger position,
//! selected match, and the current match list. The filter text is always derived
//! from the buffer content to prevent cache-drift bugs.

/// What triggered the autocomplete session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocompleteTrigger {
    /// Triggered by `#` for prompt-template completion.
    Hash,
    /// Triggered by `/` at position 0 for slash-command completion.
    Slash,
}

/// Tracks an active autocomplete session.
///
/// Lives inside [`ChatInputBoxState`](super::chat_input_box::ChatInputBoxState) as `Option<AutocompleteState>`.
/// `None` means autocomplete is not active.
///
/// The filter text is NOT stored here — it is always derived from the buffer
/// content (graphemes from `token_start + 1` to `cursor_pos`) to prevent
/// cache-drift bugs.
#[derive(Debug, Clone)]
pub struct AutocompleteState {
    /// What triggered this autocomplete session.
    pub(super) trigger: AutocompleteTrigger,
    /// Grapheme index where the trigger character (`#` or `/`) sits in the input buffer.
    pub(super) token_start: usize,
    /// Index of the currently highlighted match (0 = first in the list).
    /// The list is ordered least-relevant (index 0) to most-relevant (last index).
    pub(super) selected_index: usize,
    /// Current fuzzy matches, ordered least-relevant first, most-relevant last.
    /// Capped at 20 entries.
    pub(super) matches: Vec<crate::feat::chat_input::AutocompleteMatch>,
}

impl AutocompleteState {
    /// Returns the trigger kind for this autocomplete session.
    #[must_use]
    pub fn trigger(&self) -> AutocompleteTrigger {
        self.trigger
    }

    /// Returns the grapheme index of the trigger character.
    #[must_use]
    pub fn token_start(&self) -> usize {
        self.token_start
    }

    /// Returns the currently selected match index.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Returns the current fuzzy matches.
    #[must_use]
    pub fn matches(&self) -> &[crate::feat::chat_input::AutocompleteMatch] {
        &self.matches
    }

    /// Returns the currently selected match, if any.
    #[must_use]
    pub fn selected_match(&self) -> Option<&crate::feat::chat_input::AutocompleteMatch> {
        self.matches.get(self.selected_index)
    }

    /// Moves the selection up (toward less relevant). Clamped at 0.
    pub fn move_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    /// Moves the selection down (toward more relevant). Clamped at last entry.
    pub fn move_down(&mut self) {
        self.selected_index = self
            .selected_index
            .saturating_add(1)
            .min(self.matches.len().saturating_sub(1));
    }

    /// Replaces the match list and clamps the selected index.
    pub fn set_matches(&mut self, matches: Vec<crate::feat::chat_input::AutocompleteMatch>) {
        self.selected_index = self.selected_index.min(matches.len().saturating_sub(1));
        self.matches = matches;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use crate::feat::chat_input::AutocompleteMatch;

    fn make_match(name: &str) -> AutocompleteMatch {
        AutocompleteMatch {
            name: name.to_owned(),
            description: String::new(),
        }
    }

    fn make_state_with_3_matches() -> AutocompleteState {
        AutocompleteState {
            trigger: AutocompleteTrigger::Hash,
            token_start: 0,
            selected_index: 2, // last item (most relevant)
            matches: vec![make_match("a"), make_match("b"), make_match("c")],
        }
    }

    #[rstest::rstest]
    fn selected_index_returns_current_value() {
        // Given a state with selected_index = 2.
        let state = make_state_with_3_matches();

        // When reading selected_index.
        // Then it returns 2, not 0.
        assert_eq!(state.selected_index(), 2);
        assert_ne!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn move_up_decrements_selected_index() {
        // Given a state with selected_index = 2.
        let mut state = make_state_with_3_matches();

        // When moving up.
        state.move_up();

        // Then selected_index decrements.
        assert_eq!(state.selected_index(), 1);
    }

    #[rstest::rstest]
    fn move_up_clamps_at_zero() {
        // Given a state with selected_index = 0.
        let mut state = AutocompleteState {
            trigger: AutocompleteTrigger::Hash,
            token_start: 0,
            selected_index: 0,
            matches: vec![make_match("a")],
        };

        // When moving up.
        state.move_up();

        // Then selected_index stays at 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn move_down_increments_selected_index() {
        // Given a state with selected_index = 0.
        let mut state = AutocompleteState {
            trigger: AutocompleteTrigger::Hash,
            token_start: 0,
            selected_index: 0,
            matches: vec![make_match("a"), make_match("b"), make_match("c")],
        };

        // When moving down.
        state.move_down();

        // Then selected_index increments.
        assert_eq!(state.selected_index(), 1);
    }

    #[rstest::rstest]
    fn move_down_clamps_at_last() {
        // Given a state with selected_index at last item.
        let mut state = make_state_with_3_matches();

        // When moving down (already at last).
        state.move_down();

        // Then selected_index stays at last.
        assert_eq!(state.selected_index(), 2);
    }

    #[rstest::rstest]
    fn set_matches_updates_list() {
        // Given a state with 3 matches.
        let mut state = make_state_with_3_matches();

        // When setting new matches.
        let new_matches = vec![make_match("x"), make_match("y")];
        state.set_matches(new_matches);

        // Then the matches are updated.
        assert_eq!(state.matches.len(), 2);
        assert_eq!(state.matches[0].name, "x");
    }

    #[rstest::rstest]
    fn set_matches_clamps_selected_index() {
        // Given a state with selected_index = 2.
        let mut state = make_state_with_3_matches();

        // When setting fewer matches (selected_index would be out of bounds).
        state.set_matches(vec![make_match("only")]);

        // Then selected_index is clamped to 0.
        assert_eq!(state.selected_index(), 0);
    }

    #[rstest::rstest]
    fn set_matches_with_empty_list() {
        // Given a state.
        let mut state = make_state_with_3_matches();

        // When setting empty matches.
        state.set_matches(vec![]);

        // Then selected_index is clamped to 0.
        assert_eq!(state.selected_index(), 0);
        assert!(state.matches.is_empty());
    }

    #[rstest::rstest]
    fn selected_match_returns_correct_item() {
        // Given a state with 3 matches.
        let state = make_state_with_3_matches();

        // When reading selected_match.
        // Then it returns the item at selected_index.
        assert_eq!(state.selected_match().unwrap().name, "c");
    }
}
