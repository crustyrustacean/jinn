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
    /// Whether the tool is currently enabled for this session.
    pub enabled: bool,
    /// Theme for styling.
    pub theme: Theme,
}

impl PickerItem for ToolEntry {
    fn display_label(&self) -> &str {
        &self.name
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

        // Highlight matched bytes in the name using the same pattern as other pickers.
        let name_spans = highlight_text_with_bg(
            &self.name,
            style,
            match_indices,
            self.theme.picker_highlight_bg,
        );

        let mut spans = vec![marker_span];
        spans.extend(name_spans);
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::theme::default_theme;

    fn make_entry(name: &str, enabled: bool) -> ToolEntry {
        ToolEntry {
            name: name.to_owned(),
            description: String::new(),
            enabled,
            theme: default_theme(),
        }
    }

    #[rstest::rstest]
    fn display_label_returns_name() {
        let entry = make_entry("bash", true);
        assert_eq!(entry.display_label(), "bash");
    }

    #[rstest::rstest]
    fn render_row_enabled_shows_checkmark() {
        let entry = make_entry("bash", true);
        let line = entry.render_row(false);
        let rendered = line.to_string();
        assert!(rendered.contains('\u{2713}'), "should contain ✓");
        assert!(!rendered.contains('\u{2717}'), "should not contain ✗");
    }

    #[rstest::rstest]
    fn render_row_disabled_shows_cross() {
        let entry = make_entry("bash", false);
        let line = entry.render_row(false);
        let rendered = line.to_string();
        assert!(rendered.contains('\u{2717}'), "should contain ✗");
        assert!(!rendered.contains('\u{2713}'), "should not contain ✓");
    }
}
