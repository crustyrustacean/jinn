//! Tool picker entry type and rendering.

use crate::feat::picker::style::dim_style;
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

        // Description in dim text after an em-dash (same pattern as session lifecycle picker).
        let desc_style = dim_style(is_selected, &self.theme);
        let desc_span = Span::styled(format!(" \u{2014} {}", self.description), desc_style);

        Line::from(vec![marker_span, name_span, desc_span])
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
        let (name_indices, desc_indices) = split_match_indices(match_indices, self.name.len());

        let name_spans = highlight_text_with_bg(
            &self.name,
            style,
            &name_indices,
            self.theme.picker_highlight_bg,
        );

        // Separator and description in dim style.
        let desc_style = dim_style(is_selected, &self.theme);
        let sep_span = Span::styled(" \u{2014} ".to_owned(), desc_style);
        let desc_spans = highlight_text_with_bg(
            &self.description,
            desc_style,
            &desc_indices,
            self.theme.picker_highlight_bg,
        );

        let mut spans = vec![marker_span];
        spans.extend(name_spans);
        spans.push(sep_span);
        spans.extend(desc_spans);
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

    #[rstest::rstest]
    fn render_row_shows_description() {
        // Given a tool entry with a description.
        let entry = make_entry("bash", "Run shell commands", true);

        // When rendering the row.
        let line = entry.render_row(false);
        let rendered = line.to_string();

        // Then the description appears after an em-dash.
        assert!(rendered.contains('\u{2014}'), "should contain em-dash");
        assert!(rendered.contains("Run shell commands"));
    }

    #[rstest::rstest]
    fn render_row_description_uses_muted_color() {
        // Given a tool entry with a description.
        let entry = make_entry("bash", "Run commands", true);

        // When rendering the row (not selected).
        let line = entry.render_row(false);

        // Then the description span uses muted text color.
        let desc_span = &line.spans[2];
        assert_eq!(
            desc_span.style.fg,
            Some(default_theme().muted_text),
            "description should use muted text color"
        );
    }

    #[rstest::rstest]
    fn render_row_with_highlight_highlights_name_and_description() {
        // Given a tool entry where the filter matches both name and description.
        // search_text = "bash run shell" — name="bash" (len 4), desc="run shell"
        let entry = make_entry("bash", "run shell", true);

        // Match "sh" in name at offsets 2..4, and "sh" in description at offsets 7..9
        // (search_text = "bash run shell", desc starts at offset 5)
        let match_indices = vec![2..4, 7..9];

        // When rendering with highlight.
        let line = entry.render_row_with_highlight(false, &match_indices);

        // Then the rendered output contains both the name and description.
        let rendered = line.to_string();
        assert!(
            rendered.contains("bash"),
            "row should contain tool name"
        );
        assert!(
            rendered.contains("run shell"),
            "row should contain tool description"
        );
    }
}
