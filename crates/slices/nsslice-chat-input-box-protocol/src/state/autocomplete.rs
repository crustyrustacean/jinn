//! Autocomplete state for prompt-template completion.
//!
//! Tracks the active autocomplete session: trigger position, selected match,
//! and the current match list. The filter text is always derived from the
//! buffer content to prevent cache-drift bugs.

/// Tracks an active prompt-template autocomplete session.
///
/// Lives inside [`ChatInputBoxState`](super::ChatInputBoxState) as `Option<AutocompleteState>`.
/// `None` means autocomplete is not active.
///
/// The filter text is NOT stored here — it is always derived from the buffer
/// content (graphemes from `token_start + 1` to `cursor_pos`) to prevent
/// cache-drift bugs.
#[derive(Debug, Clone)]
pub struct AutocompleteState {
    /// Grapheme index where the `$` trigger character sits in the input buffer.
    pub(super) token_start: usize,
    /// Index of the currently highlighted match (0 = first in the list).
    /// The list is ordered least-relevant (index 0) to most-relevant (last index).
    pub(super) selected_index: usize,
    /// Current fuzzy matches, ordered least-relevant first, most-relevant last.
    /// Capped at 20 entries.
    pub(super) matches: Vec<crate::AutocompleteMatch>,
}

impl AutocompleteState {
    /// Returns the grapheme index of the `$` trigger.
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
    pub fn matches(&self) -> &[crate::AutocompleteMatch] {
        &self.matches
    }

    /// Returns the currently selected match, if any.
    #[must_use]
    pub fn selected_match(&self) -> Option<&crate::AutocompleteMatch> {
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
    pub fn set_matches(&mut self, matches: Vec<crate::AutocompleteMatch>) {
        self.selected_index = self.selected_index.min(matches.len().saturating_sub(1));
        self.matches = matches;
    }
}
