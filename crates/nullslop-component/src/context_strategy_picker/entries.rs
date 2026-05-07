//! Strategy entries for the context assembly picker.
//!
//! Builds the list of available strategies from the [`StrategyRegistryService`],
//! and implements [`PickerItem`] so [`SelectionState`] can fuzzy-filter
//! and render them. Also provides footer formatting for the strategy picker overlay.
//!
//! [`PickerItem`]: nullslop_selection_widget::PickerItem
//! [`SelectionState`]: nullslop_selection_widget::SelectionState

use std::ops::Range;

use crate::PICKER_HIGHLIGHT_STYLE;
use crate::AppState;
use nullslop_protocol::PromptStrategyId;
use nullslop_selection_widget::PickerItem;
use nullslop_services::Services;
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
        render_strategy_row(&self.name, &self.description, self.is_active, is_selected, &[])
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

    // Highlight within the name portion only.
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

/// Splits `text` into spans, applying the highlight style to characters whose
/// byte offset falls within one of `match_indices`.
///
/// Matched characters get [`PICKER_HIGHLIGHT_STYLE`] patched onto the base style
/// (preserving the base foreground color).
fn highlight_text<'a>(
    text: &str,
    base_style: Style,
    match_indices: &[Range<usize>],
) -> Vec<Span<'a>> {
    if match_indices.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_owned(), base_style)];
    }

    let highlight_style = base_style.patch(PICKER_HIGHLIGHT_STYLE);

    let mut spans = Vec::new();
    let mut current_start = 0;
    let mut in_highlight = false;

    for (byte_off, _ch) in text.char_indices() {
        let is_matched = match_indices.iter().any(|r| r.contains(&byte_off));

        if is_matched != in_highlight {
            let segment = text[current_start..byte_off].to_owned();
            if !segment.is_empty() {
                spans.push(Span::styled(
                    segment,
                    if in_highlight { highlight_style } else { base_style },
                ));
            }
            current_start = byte_off;
            in_highlight = is_matched;
        }
    }

    if current_start < text.len() {
        let rest = text[current_start..].to_owned();
        spans.push(Span::styled(
            rest,
            if in_highlight { highlight_style } else { base_style },
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), base_style));
    }

    spans
}

/// Loads strategy entries from the strategy registry, marking the active one.
///
/// Reads available strategies from `services.strategy_registry`, maps them to
/// [`StrategyEntry`], and marks the one matching `active_strategy` as `is_active: true`.
pub fn load_strategy_entries(
    services: &Services,
    active_strategy: &PromptStrategyId,
) -> Vec<StrategyEntry> {
    let strategies = services.strategy_registry.list();
    strategies
        .into_iter()
        .map(|info| {
            let is_active = &info.id == active_strategy;
            StrategyEntry {
                strategy_id: info.id,
                is_active,
                name: info.name,
                description: info.description,
            }
        })
        .collect()
}

/// Reorders strategy entries so the active strategy is promoted to the top
/// when the filter is empty. When the filter is non-empty, preserves fuzzy match order.
pub fn sorted_strategy_entries(
    entries: &[StrategyEntry],
    filter: &str,
) -> Vec<StrategyEntry> {
    let mut result = entries.to_vec();

    if filter.is_empty()
        && let Some(pos) = result.iter().position(|e| e.is_active)
        && pos > 0
    {
        #[expect(
            clippy::indexing_slicing,
            reason = "pos comes from iter().position() on the same vec"
        )]
        result[0..=pos].rotate_right(1);
    }

    result
}

/// Loads strategy entries into the picker state, ready for display.
///
/// Reads from the strategy registry, marks the active strategy from the
/// current session, applies active-first sorting, then stores the entries
/// via [`SelectionState::set_items`].
///
/// [`SelectionState`]: nullslop_selection_widget::SelectionState
pub fn load_strategy_picker_items(services: &Services, state: &mut AppState) {
    let active_strategy = state.active_session().active_strategy().clone();
    let all = load_strategy_entries(services, &active_strategy);
    let entries = sorted_strategy_entries(&all, "");
    state.context_strategy_picker.set_items(entries);
}

/// Formats the footer line showing the current strategy.
///
/// Returns a styled [`Line`] showing "Current: `strategy_name`" with the label
/// in dark gray and the name in white.
pub fn format_strategy_footer(strategy_name: &str) -> ratatui::text::Line<'static> {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    let gray = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled("Current: ".to_owned(), gray),
        Span::styled(
            strategy_name.to_owned(),
            Style::default().fg(Color::White),
        ),
    ])
}

#[cfg(test)]
#[path = "entries_tests.rs"]
mod entries_tests;
