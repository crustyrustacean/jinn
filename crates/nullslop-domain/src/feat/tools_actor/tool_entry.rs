//! Tool picker entry type and rendering.

use crate::feat::theme::Theme;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight::highlight_text_with_bg;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::ops::Range;

/// A tool entry ready for display in the tool picker.
#[derive(Debug, Clone)]
pub struct ToolEntry {
    /// Tool name (unique identifier, e.g., "bash", "edit").
    pub name: String,
    /// Human-readable tool description.
    pub description: String,
    /// Combined searchable text: `"{name} {description}"`.
    /// Used for fuzzy matching so users can search by description terms.
    pub search_text: String,
    /// Whether the tool is currently enabled for this session.
    pub enabled: bool,
    /// Theme for styling.
    pub theme: Theme,
}

impl PickerItem for ToolEntry {
    fn display_label(&self) -> &str {
        &self.search_text
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        let style = if is_selected {
            Style::default()
                .fg(self.theme.primary_text)
                .bg(self.theme.picker_selected_bg)
        } else {
            Style::default()
        };

        let (marker, marker_color) = if self.enabled {
            ("\u{2713} ", self.theme.focus_accent) // ✓
        } else {
            ("\u{2717} ", self.theme.error_text) // ✗
        };

        let marker_span = Span::styled(marker.to_owned(), Style::default().fg(marker_color));
        let name_span = Span::styled(self.name.clone(), style);
        Line::from(vec![marker_span, name_span])
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        let style = if is_selected {
            Style::default()
                .fg(self.theme.primary_text)
                .bg(self.theme.picker_selected_bg)
        } else {
            Style::default()
        };

        let (marker, marker_color) = if self.enabled {
            ("\u{2713} ", self.theme.focus_accent)
        } else {
            ("\u{2717} ", self.theme.error_text)
        };

        let marker_span = Span::styled(marker.to_owned(), Style::default().fg(marker_color));

        // Match indices are byte offsets into search_text = "{name} {description}".
        // Only highlight the name portion in the row for now (Phase 3 adds inline description).
        let (name_indices, _desc_indices) = split_match_indices(match_indices, self.name.len());

        let name_spans = highlight_text_with_bg(
            &self.name,
            style,
            &name_indices,
            self.theme.picker_highlight_bg,
        );

        let mut spans = vec![marker_span];
        spans.extend(name_spans);
        Line::from(spans)
    }
}

/// Splits match indices from `search_text = "{name} {description}"` into
/// name-portion and description-portion indices.
///
/// The space separator occupies byte offset `name_len`. Description indices
/// are remapped to be relative to the start of the description string.
fn split_match_indices(
    indices: &[Range<usize>],
    name_len: usize,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let desc_offset = name_len + 1;

    let mut name_indices = Vec::new();
    let mut desc_indices = Vec::new();

    for range in indices {
        if range.start < name_len {
            let end = range.end.min(name_len);
            name_indices.push(range.start..end);
        }

        if range.end > desc_offset {
            let start = range.start.saturating_sub(desc_offset);
            let end = range.end.saturating_sub(desc_offset);
            desc_indices.push(start..end);
        }
    }

    (name_indices, desc_indices)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::single_range_in_vec_init,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::theme::default_theme;

    fn make_entry(name: &str, description: &str, enabled: bool) -> ToolEntry {
        ToolEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            search_text: format!("{name} {description}"),
            enabled,
            theme: default_theme(),
        }
    }

    #[rstest::rstest]
    fn display_label_returns_search_text() {
        // Given a tool entry with name and description.
        let entry = make_entry("bash", "Run shell commands", true);

        // When getting the display label.
        let label = entry.display_label();

        // Then it contains both name and description.
        assert_eq!(label, "bash Run shell commands");
    }

    #[rstest::rstest]
    fn render_row_enabled_shows_checkmark() {
        let entry = make_entry("bash", "Run commands", true);
        let line = entry.render_row(false);
        let rendered = line.to_string();
        assert!(rendered.contains('\u{2713}'), "should contain ✓");
        assert!(!rendered.contains('\u{2717}'), "should not contain ✗");
    }

    #[rstest::rstest]
    fn render_row_disabled_shows_cross() {
        let entry = make_entry("bash", "Run commands", false);
        let line = entry.render_row(false);
        let rendered = line.to_string();
        assert!(rendered.contains('\u{2717}'), "should contain ✗");
        assert!(!rendered.contains('\u{2713}'), "should not contain ✓");
    }

    #[rstest::rstest]
    fn split_match_indices_partitions_correctly() {
        // Given match indices spanning across name/description boundary.
        // search_text = "abc xyz" (name="abc", desc="xyz", separator at byte 3)
        let (name_idx, desc_idx) = split_match_indices(&[0..5], 3);

        // Then name gets [0..3] and description gets [0..1].
        assert_eq!(name_idx, vec![0..3]);
        assert_eq!(desc_idx, vec![0..1]);
    }

    #[rstest::rstest]
    fn split_match_indices_name_only_match() {
        let (name_idx, desc_idx) = split_match_indices(&[0..2], 5);

        assert_eq!(name_idx, vec![0..2]);
        assert!(desc_idx.is_empty());
    }

    #[rstest::rstest]
    fn split_match_indices_description_only_match() {
        let (name_idx, desc_idx) = split_match_indices(&[6..11], 5);

        assert!(name_idx.is_empty());
        assert_eq!(desc_idx, vec![0..5]);
    }
}
