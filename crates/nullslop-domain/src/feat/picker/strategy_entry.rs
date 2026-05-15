//! Context strategy picker entry type and rendering.

use std::ops::Range;

use super::style::{active_marker, dim_style, selected_style};
use crate::feat::theme::Theme;
use crate::protocol::PromptStrategyId;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text_with_bg;
use ratatui::text::{Line, Span};

/// A context assembly strategy entry ready for display in the picker.
#[derive(Debug, Clone)]
pub struct StrategyEntry {
    /// The unique strategy identifier.
    pub strategy_id: PromptStrategyId,
    /// Human-readable display name (e.g., "Sliding Window").
    pub name: String,
    /// Short description of what the strategy does.
    pub description: String,
    /// Whether this is the currently active strategy for the session.
    pub is_active: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

impl PickerItem for StrategyEntry {
    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_strategy_row(
            &self.name,
            &self.description,
            self.is_active,
            is_selected,
            &[],
            &self.theme,
        )
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_strategy_row(
            &self.name,
            &self.description,
            self.is_active,
            is_selected,
            match_indices,
            &self.theme,
        )
    }
}

/// Renders a strategy picker row, optionally highlighting matched characters.
///
/// Match indices are byte offsets into `name` (the `display_label`).
fn render_strategy_row(
    name: &str,
    description: &str,
    is_active: bool,
    is_selected: bool,
    match_indices: &[Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let active_marker = active_marker(is_active, theme);

    let name_style = selected_style(is_selected, theme);

    let desc_style = dim_style(is_selected, theme);

    let name_spans = if match_indices.is_empty() {
        vec![Span::styled(format!("{name}  "), name_style)]
    } else {
        let mut spans =
            highlight_text_with_bg(name, name_style, match_indices, theme.picker_highlight_bg);
        spans.push(Span::styled("  ".to_owned(), name_style));
        spans
    };

    let mut all_spans = vec![active_marker];
    all_spans.extend(name_spans);
    all_spans.push(Span::styled(description.to_owned(), desc_style));
    Line::from(all_spans)
}
