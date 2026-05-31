//! Session lifecycle picker entry - one row in the lifecycle selection picker.

use jinn_selection_widget::PickerItem;
use ratatui::text::{Line, Span};

use crate::feat::picker::style::{active_marker, dim_style, selected_style};
use crate::feat::theme::Theme;

/// A lifecycle recipe shown in the session lifecycle picker.
#[derive(Debug, Clone)]
pub struct SessionLifecycleEntry {
    /// The lifecycle name (or "blank" for the implicit default).
    pub name: String,
    /// Optional description shown below the name.
    pub description: Option<String>,
    /// Whether this lifecycle requires user-provided args (`$1`, `$2`, etc.).
    pub has_args: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

impl PickerItem for SessionLifecycleEntry {
    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_lifecycle_row(
            &self.name,
            self.description.as_deref(),
            self.has_args,
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
        render_lifecycle_row(
            &self.name,
            self.description.as_deref(),
            self.has_args,
            is_selected,
            match_indices,
            &self.theme,
        )
    }
}

/// Renders a lifecycle picker row with proper styling.
fn render_lifecycle_row(
    name: &str,
    description: Option<&str>,
    has_args: bool,
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

    if has_args {
        spans.push(Span::styled(" *".to_owned(), desc_style));
    }

    if let Some(desc) = description {
        spans.push(Span::styled(format!(" - {desc}"), desc_style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::theme::default_theme;

    fn test_entry(name: &str, description: Option<&str>, has_args: bool) -> SessionLifecycleEntry {
        SessionLifecycleEntry {
            name: name.to_owned(),
            description: description.map(String::from),
            has_args,
            theme: default_theme(),
        }
    }

    #[rstest::rstest]
    fn display_label_returns_name() {
        let entry = test_entry("fossil branch", None, false);
        assert_eq!(entry.display_label(), "fossil branch");
    }

    #[rstest::rstest]
    fn render_row_unselected_has_spaces() {
        let entry = test_entry("blank", None, false);
        let line = entry.render_row(false);
        let text = line.to_string();
        assert!(text.starts_with("  blank"));
    }

    #[rstest::rstest]
    fn render_row_selected_has_arrow() {
        let entry = test_entry("blank", None, false);
        let line = entry.render_row(true);
        let text = line.to_string();
        assert!(text.starts_with("> blank"));
    }

    #[rstest::rstest]
    fn render_row_shows_args_indicator() {
        let entry = test_entry("fossil branch", None, true);
        let line = entry.render_row(false);
        let text = line.to_string();
        assert!(text.contains('*'));
    }

    #[rstest::rstest]
    fn render_row_shows_description() {
        let entry = test_entry("fossil branch", Some("Open a fossil branch"), false);
        let line = entry.render_row(false);
        let text = line.to_string();
        assert!(text.contains("Open a fossil branch"));
    }
}
