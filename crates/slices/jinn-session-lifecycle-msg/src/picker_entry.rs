//! Session lifecycle picker entry - one row in the lifecycle selection picker.

use jinn_picker::RowCtx;
use jinn_picker::picker_style::{active_marker, dim_style, selected_style};
use jinn_selection_widget::highlight_text_with_bg;
use jinn_theme::Theme;
use ratatui::text::{Line, Span};

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

impl jinn_selection_widget::TreeItem for SessionLifecycleEntry {
    fn id(&self) -> &str {
        &self.name
    }

    fn parent_id(&self) -> Option<&str> {
        None
    }

    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, _is_selected: bool) -> ratatui::text::Line<'static> {
        // Rows render through the spec's row hook via PickerEntry; this
        // impl only supplies tree structure (id/parent_id) and filter text.
        ratatui::text::Line::raw(self.display_label().to_owned())
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> ratatui::text::Line<'static> {
        ratatui::text::Line::raw(self.display_label().to_owned())
    }
}

/// Renders one picker row: the cursor marker, the lifecycle name, a ` *`
/// marker when the setup command needs user-supplied args, and the
/// description after an em-dash separator.
pub fn lifecycle_row(entry: &SessionLifecycleEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    let base_style = selected_style(ctx.is_selected, &entry.theme);
    let desc_style = dim_style(ctx.is_selected, &entry.theme);

    let mut spans = vec![active_marker(ctx.is_selected, &entry.theme)];

    if ctx.match_ranges.is_empty() {
        spans.push(Span::styled(entry.name.clone(), base_style));
    } else {
        spans.extend(highlight_text_with_bg(
            &entry.name,
            base_style,
            ctx.match_ranges,
            entry.theme.picker_highlight_bg,
        ));
    }

    if entry.has_args {
        spans.push(Span::styled(" *".to_owned(), desc_style));
    }

    if let Some(desc) = &entry.description {
        spans.push(Span::styled(format!(" \u{2014} {desc}"), desc_style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_picker::RowCtx;
    use jinn_theme::default_theme;

    fn test_entry(name: &str, description: Option<&str>, has_args: bool) -> SessionLifecycleEntry {
        SessionLifecycleEntry {
            name: name.to_owned(),
            description: description.map(String::from),
            has_args,
            theme: default_theme(),
        }
    }

    fn row_ctx(ranges: &[std::ops::Range<usize>], is_selected: bool) -> RowCtx<'_> {
        RowCtx::flat(is_selected, ranges)
    }

    fn no_matches() -> Vec<std::ops::Range<usize>> {
        Vec::new()
    }

    #[rstest::rstest]
    fn row_unselected_has_spaces() {
        let entry = test_entry("blank", None, false);
        let ranges = no_matches();
        let ctx = row_ctx(&ranges, false);
        let line = lifecycle_row(&entry, &ctx);
        let text = line.to_string();
        assert!(text.starts_with("  blank"));
    }

    #[rstest::rstest]
    fn row_selected_has_arrow() {
        let entry = test_entry("blank", None, false);
        let ranges = no_matches();
        let ctx = row_ctx(&ranges, true);
        let line = lifecycle_row(&entry, &ctx);
        let text = line.to_string();
        assert!(text.starts_with("> blank"));
    }

    #[rstest::rstest]
    fn row_shows_args_indicator() {
        let entry = test_entry("fossil branch", None, true);
        let ranges = no_matches();
        let ctx = row_ctx(&ranges, false);
        let line = lifecycle_row(&entry, &ctx);
        let text = line.to_string();
        assert!(text.contains('*'));
    }

    #[rstest::rstest]
    fn row_shows_description() {
        let entry = test_entry("fossil branch", Some("Open a fossil branch"), false);
        let ranges = no_matches();
        let ctx = row_ctx(&ranges, false);
        let line = lifecycle_row(&entry, &ctx);
        let text = line.to_string();
        assert!(text.contains("Open a fossil branch"));
    }
}
