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
