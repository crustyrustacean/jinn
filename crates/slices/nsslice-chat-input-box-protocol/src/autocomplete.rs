//! Prompt-template autocomplete subsystem for the chat input box.
//!
//! Provides [`AutocompleteMatch`] for popup rendering, [`AutocompleteState`] for
//! tracking the active session, and autocomplete methods on [`ChatInputBoxState`].

use unicode_segmentation::UnicodeSegmentation as _;

use super::ChatInputBoxState;

/// A single match shown in the autocomplete popup.
///
/// Lightweight snapshot — stores only the name and description for rendering.
/// The full template body is looked up from the store only when needed
/// (e.g. double-`$` expansion).
#[derive(Debug, Clone)]
pub struct AutocompleteMatch {
    /// The template name (e.g. `"code-review"`).
    pub name: String,
    /// Short human-readable description for the popup.
    pub description: String,
}

/// Tracks an active prompt-template autocomplete session.
///
/// Lives inside [`ChatInputBoxState`] as `Option<AutocompleteState>`.
/// `None` means autocomplete is not active.
///
/// The filter text is NOT stored here — it is always derived from the buffer
/// content (graphemes from `token_start + 1` to `cursor_pos`) to prevent
/// cache-drift bugs.
#[derive(Debug, Clone)]
pub struct AutocompleteState {
    /// Grapheme index where the `$` trigger character sits in the input buffer.
    token_start: usize,
    /// Index of the currently highlighted match (0 = first in the list).
    /// The list is ordered least-relevant (index 0) to most-relevant (last index).
    selected_index: usize,
    /// Current fuzzy matches, ordered least-relevant first, most-relevant last.
    /// Capped at 20 entries.
    matches: Vec<AutocompleteMatch>,
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
    pub fn matches(&self) -> &[AutocompleteMatch] {
        &self.matches
    }

    /// Returns the currently selected match, if any.
    #[must_use]
    pub fn selected_match(&self) -> Option<&AutocompleteMatch> {
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
    pub fn set_matches(&mut self, matches: Vec<AutocompleteMatch>) {
        self.selected_index = self.selected_index.min(matches.len().saturating_sub(1));
        self.matches = matches;
    }
}

// ---------------------------------------------------------------------------
// Autocomplete methods on ChatInputBoxState
// ---------------------------------------------------------------------------

impl ChatInputBoxState {
    /// Returns a reference to the active autocomplete state, if any.
    #[must_use]
    pub fn autocomplete(&self) -> &Option<AutocompleteState> {
        &self.autocomplete
    }

    /// Returns a mutable reference to the active autocomplete state.
    #[must_use]
    pub fn autocomplete_mut(&mut self) -> &mut Option<AutocompleteState> {
        &mut self.autocomplete
    }

    /// Deactivates autocomplete (dismisses the popup).
    pub fn deactivate_autocomplete(&mut self) {
        self.autocomplete = None;
    }

    /// Activates autocomplete at the given grapheme index (where `$` was typed).
    ///
    /// If `matches` is empty, `selected_index` is set to 0.
    /// If non-empty, `selected_index` defaults to the last entry (most relevant).
    pub fn activate_autocomplete(&mut self, token_start: usize, matches: Vec<AutocompleteMatch>) {
        let selected_index = if matches.is_empty() {
            0
        } else {
            matches.len() - 1
        };
        self.autocomplete = Some(AutocompleteState {
            token_start,
            selected_index,
            matches,
        });
    }

    /// Updates the match list in the active autocomplete state, clamping selection.
    pub fn update_autocomplete_matches(&mut self, matches: Vec<AutocompleteMatch>) {
        if let Some(ac) = &mut self.autocomplete {
            ac.set_matches(matches);
        }
    }

    /// Returns the current filter text derived from the buffer content.
    ///
    /// Extracts graphemes from `token_start + 1` to `cursor_pos`.
    /// Returns `None` if autocomplete is not active.
    #[must_use]
    pub fn autocomplete_filter(&self) -> Option<String> {
        let ac = self.autocomplete.as_ref()?;
        let start = ac.token_start + 1;
        let end = self.cursor_pos;
        if start >= end {
            return Some(String::new());
        }
        let filter: String = self
            .input_buffer
            .graphemes(true)
            .enumerate()
            .skip_while(|(i, _)| *i < start)
            .take_while(|(i, _)| *i < end)
            .map(|(_, g)| g)
            .collect();
        Some(filter)
    }

    /// Returns the currently selected autocomplete match.
    #[must_use]
    pub fn autocomplete_selected(&self) -> Option<&AutocompleteMatch> {
        self.autocomplete.as_ref()?.selected_match()
    }

    /// Moves the autocomplete selection up (toward less relevant).
    pub fn autocomplete_move_up(&mut self) {
        if let Some(ac) = &mut self.autocomplete {
            ac.move_up();
        }
    }

    /// Moves the autocomplete selection down (toward more relevant).
    pub fn autocomplete_move_down(&mut self) {
        if let Some(ac) = &mut self.autocomplete {
            ac.move_down();
        }
    }

    /// Returns the `token_start` grapheme index if autocomplete is active.
    #[must_use]
    pub fn autocomplete_token_start(&self) -> Option<usize> {
        self.autocomplete.as_ref().map(|ac| ac.token_start)
    }

    /// Returns the screen column of the autocomplete `$` trigger within its visual line.
    ///
    /// The column is a grapheme offset within the line that contains the `$`.
    /// Returns `None` if autocomplete is not active.
    #[must_use]
    pub fn autocomplete_token_screen_col(&self) -> Option<usize> {
        let ac = self.autocomplete.as_ref()?;
        let token_start = ac.token_start();
        let mut col = 0;
        for (i, g) in self.input_buffer.graphemes(true).enumerate() {
            if i == token_start {
                return Some(col);
            }
            if g == "\n" {
                col = 0;
            } else {
                col += 1;
            }
        }
        Some(col) // token_start at end of buffer
    }

    /// Completes the autocomplete: replaces the `$partial` region with `$name`.
    ///
    /// The region replaced is `token_start..cursor_pos` (grapheme indices).
    /// After completion, the cursor lands after the completed text,
    /// and autocomplete remains active with updated filter.
    pub fn complete_autocomplete(&mut self, name: &str) {
        let Some(ac) = self.autocomplete.as_ref() else {
            return;
        };
        let token_start = ac.token_start;
        let replacement = format!("${name}");
        self.replace_grapheme_range(token_start, self.cursor_pos, &replacement);
        // Cursor is now after the replacement text.
        // Autocomplete stays active — the filter is now the exact name.
    }

    /// Expands a double-`$` token: replaces `$name$` with the template body.
    ///
    /// The region replaced is `token_start..cursor_pos` (which includes `$name$`).
    /// After expansion, autocomplete is deactivated and the cursor lands after
    /// the body text.
    pub fn expand_autocomplete(&mut self, body: &str) {
        let Some(ac) = self.autocomplete.as_ref() else {
            return;
        };
        let token_start = ac.token_start;
        self.replace_grapheme_range(token_start, self.cursor_pos, body);
        self.autocomplete = None;
    }
}
