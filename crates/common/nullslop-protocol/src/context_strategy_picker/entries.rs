//! Context strategy picker entry type and rendering.

use std::ops::Range;

use crate::PromptStrategyId;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text;
use ratatui::style::{Color, Modifier, Style};
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
) -> Line<'static> {
    let active_marker = Span::styled(
        if is_active { "> " } else { "  " },
        if is_active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    );

    let name_style = if is_selected {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default()
    };

    let desc_style = if is_selected {
        Style::default().fg(Color::DarkGray).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let name_spans = if match_indices.is_empty() {
        vec![Span::styled(format!("{name}  "), name_style)]
    } else {
        let mut spans = highlight_text(name, name_style, match_indices);
        spans.push(Span::styled("  ".to_owned(), name_style));
        spans
    };

    let mut all_spans = vec![active_marker];
    all_spans.extend(name_spans);
    all_spans.push(Span::styled(description.to_owned(), desc_style));
    Line::from(all_spans)
}
