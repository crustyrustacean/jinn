//! Fork picker entry type and rendering.
//!
//! Each [`ForkEntry`] represents a User or Assistant chat entry that can be
//! selected as the fork point. Entries are color-coded: User entries use the
//! theme's `user_block_bg`, Assistant entries use default styling.

use std::ops::Range;

use crate::feat::picker::style::{dim_style, selected_style};
use crate::feat::theme::Theme;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text_with_bg;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// A chat entry displayed in the fork picker.
///
/// Carries the entry's ordinal (position in history), full text for fuzzy
/// matching, and kind metadata for color-coded rendering.
#[derive(Debug, Clone)]
pub struct ForkEntry {
    /// The entry's position (0-based) in the session history.
    pub ordinal: usize,
    /// Full text content for fuzzy matching and display.
    pub text: String,
    /// Whether this is a user entry (affects color coding).
    pub is_user: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

impl PickerItem for ForkEntry {
    fn display_label(&self) -> &str {
        &self.text
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_fork_row(&self.text, self.is_user, is_selected, &[], &self.theme)
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_fork_row(
            &self.text,
            self.is_user,
            is_selected,
            match_indices,
            &self.theme,
        )
    }
}

/// Renders a fork picker row.
///
/// User entries get the `user_block_bg` background; Assistant entries use
/// default styling. A prefix label ("You" or "Asst") identifies the kind.
fn render_fork_row(
    text: &str,
    is_user: bool,
    is_selected: bool,
    match_indices: &[Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let base_style = if is_user {
        Style::default()
            .bg(theme.user_block_bg)
            .fg(theme.primary_text)
    } else {
        selected_style(is_selected, theme)
    };

    let prefix = if is_user { "You:  " } else { "Asst: " };
    let prefix_style = dim_style(is_selected, theme);

    let text_spans = if match_indices.is_empty() {
        vec![Span::styled(text.to_owned(), base_style)]
    } else {
        highlight_text_with_bg(text, base_style, match_indices, theme.picker_highlight_bg)
    };

    let mut spans = vec![Span::styled(prefix.to_owned(), prefix_style)];
    spans.extend(text_spans);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::theme::default_theme;

    #[rstest::rstest]
    fn fork_entry_display_label_returns_text() {
        // Given a ForkEntry with text.
        let entry = ForkEntry {
            ordinal: 0,
            text: "Hello world".to_owned(),
            is_user: true,
            theme: default_theme(),
        };

        // When calling display_label.
        // Then it returns the text.
        assert_eq!(entry.display_label(), "Hello world");
    }

    #[rstest::rstest]
    fn render_user_entry_contains_text() {
        // Given a user fork entry.
        let entry = ForkEntry {
            ordinal: 0,
            text: "My question".to_owned(),
            is_user: true,
            theme: default_theme(),
        };

        // When rendering.
        let row = entry.render_row(false);

        // Then the text appears in the rendered line.
        assert!(row.spans.iter().any(|s| s.content.contains("My question")));
        // And the prefix "You" appears.
        assert!(row.spans.iter().any(|s| s.content.contains("You")));
    }

    #[rstest::rstest]
    fn render_assistant_entry_contains_text() {
        // Given an assistant fork entry.
        let entry = ForkEntry {
            ordinal: 1,
            text: "My response".to_owned(),
            is_user: false,
            theme: default_theme(),
        };

        // When rendering.
        let row = entry.render_row(false);

        // Then the text appears in the rendered line.
        assert!(row.spans.iter().any(|s| s.content.contains("My response")));
        // And the prefix "Asst" appears.
        assert!(row.spans.iter().any(|s| s.content.contains("Asst")));
    }
}
