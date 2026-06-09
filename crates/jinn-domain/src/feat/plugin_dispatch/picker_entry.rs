//! Plugin picker entry - one row in the plugin selection picker.

use jinn_selection_widget::PickerItem;
use ratatui::text::{Line, Span};

use crate::feat::picker::style::{active_marker, dim_style, selected_style};
use crate::feat::theme::Theme;

/// An attachable plugin shown in the plugin picker.
#[derive(Debug, Clone)]
pub struct PluginPickerEntry {
    /// The Lua plugin name (used to locate the `init.lua` script at spawn time).
    pub name: String,
    /// Optional description parsed from the plugin's `-- description:` header comment.
    pub description: Option<String>,
    /// Theme for rendering.
    pub theme: Theme,
}

impl PickerItem for PluginPickerEntry {
    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_plugin_row(
            &self.name,
            self.description.as_deref(),
            is_selected,
            &[],
            &self.theme,
        )
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[std::ops::Range<usize>],
    ) -> Line<'static> {
        render_plugin_row(
            &self.name,
            self.description.as_deref(),
            is_selected,
            match_indices,
            &self.theme,
        )
    }
}

/// Renders a plugin picker row with proper styling.
fn render_plugin_row(
    name: &str,
    description: Option<&str>,
    is_selected: bool,
    match_indices: &[std::ops::Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let base_style = selected_style(is_selected, theme);
    let desc_style = dim_style(is_selected, theme);

    let mut spans = vec![active_marker(is_selected, theme)];

    if match_indices.is_empty() {
        spans.push(Span::styled(name.to_owned(), base_style));
    } else {
        spans.extend(jinn_selection_widget::highlight_text_with_bg(
            name,
            base_style,
            match_indices,
            theme.picker_highlight_bg,
        ));
    }

    if let Some(desc) = description {
        spans.push(Span::styled(format!(" \u{2014} {desc}"), desc_style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::theme::default_theme;

    fn test_entry(name: &str, description: Option<&str>) -> PluginPickerEntry {
        PluginPickerEntry {
            name: name.to_owned(),
            description: description.map(String::from),
            theme: default_theme(),
        }
    }

    #[test]
    fn display_label_returns_name() {
        // Given a plugin picker entry.
        let entry = test_entry("add-numbers", None);

        // When reading the display label.
        // Then it returns the plugin name.
        assert_eq!(entry.display_label(), "add-numbers");
    }

    #[rstest::rstest]
    fn render_row_unselected_has_spaces() {
        // Given an unselected entry.
        let entry = test_entry("add-numbers", None);

        // When rendering the row.
        let line = entry.render_row(false);
        let text = line.to_string();

        // Then it starts with spaces (no arrow).
        assert!(text.starts_with("  add-numbers"));
    }

    #[rstest::rstest]
    fn render_row_selected_has_arrow() {
        // Given a selected entry.
        let entry = test_entry("add-numbers", None);

        // When rendering the row.
        let line = entry.render_row(true);
        let text = line.to_string();

        // Then it starts with an arrow marker.
        assert!(text.starts_with("> add-numbers"));
    }

    #[rstest::rstest]
    fn render_row_shows_description() {
        // Given an entry with a description.
        let entry = test_entry("add-numbers", Some("Adds two numbers"));

        // When rendering the row.
        let line = entry.render_row(false);
        let text = line.to_string();

        // Then the description is shown.
        assert!(text.contains("Adds two numbers"));
    }

    #[rstest::rstest]
    fn render_row_without_description() {
        // Given an entry without a description.
        let entry = test_entry("add-numbers", None);

        // When rendering the row.
        let line = entry.render_row(false);
        let text = line.to_string();

        // Then only the name is shown (no em-dash separator).
        assert_eq!(text.trim(), "add-numbers");
    }
}
