//! Reasoning effort picker entry type and rendering.

use jinn_picker::RowCtx;
use jinn_picker::picker_style::dim_style;
use jinn_selection_widget::highlight_text_with_bg;
use jinn_theme::Theme;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::ReasoningEffort;

/// A reasoning effort entry ready for display in the picker.
///
/// Carries the [`ReasoningEffort`] variant so the confirm handler can read it
/// back without re-parsing the display name.
#[derive(Debug, Clone)]
pub struct ReasoningEffortEntry {
    /// The effort variant this entry represents.
    pub effort: ReasoningEffort,
    /// Human-readable display name (the wire string, e.g. "high").
    pub name: String,
    /// Short human description (e.g. "High effort").
    pub description: String,
    /// Whether this is the currently resolved effort.
    pub is_active: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

/// Renders one reasoning picker row through the picker framework's row hook.
///
/// The single renderer both the picker and any future reader supply, so
/// filtering, selection, and highlight rendering cannot drift.
pub fn reasoning_row(entry: &ReasoningEffortEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    let active_marker = Span::styled(
        if entry.is_active { "> " } else { "  " },
        if entry.is_active {
            Style::default()
                .fg(entry.theme.picker_active_marker)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    );

    let name_style = if ctx.is_selected {
        Style::default()
            .fg(entry.theme.primary_text)
            .bg(entry.theme.picker_selected_bg)
    } else {
        Style::default()
    };

    let desc_style = dim_style(ctx.is_selected, &entry.theme);

    let name_spans = if ctx.match_ranges.is_empty() {
        vec![Span::styled(format!("{}  ", entry.name), name_style)]
    } else {
        let mut spans = highlight_text_with_bg(
            &entry.name,
            name_style,
            ctx.match_ranges,
            entry.theme.picker_highlight_bg,
        );
        spans.push(Span::styled("  ".to_owned(), name_style));
        spans
    };

    let mut all_spans = vec![active_marker];
    all_spans.extend(name_spans);
    all_spans.push(Span::styled(entry.description.clone(), desc_style));
    Line::from(all_spans)
}

impl jinn_selection_widget::TreeItem for ReasoningEffortEntry {
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
        // Rows render through the picker's row hook via PickerEntry; this
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
