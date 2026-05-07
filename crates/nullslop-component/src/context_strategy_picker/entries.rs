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
mod tests {
    use nullslop_selection_widget::PickerItem as _;

    use super::*;

    fn make_entry(id: &str, name: &str, description: &str, is_active: bool) -> StrategyEntry {
        StrategyEntry {
            strategy_id: PromptStrategyId::new(id),
            name: name.to_owned(),
            description: description.to_owned(),
            is_active,
        }
    }

    // --- PickerItem tests ---

    #[test]
    fn render_row_shows_active_marker_when_active() {
        // Given a strategy entry that is active.
        let entry = make_entry("passthrough", "Passthrough", "desc", true);

        // When rendering the row (not selected).
        let line = entry.render_row(false);

        // Then the rendered line contains ">".
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains('>'));
    }

    #[test]
    fn render_row_shows_no_marker_when_inactive() {
        // Given a strategy entry that is not active.
        let entry = make_entry("passthrough", "Passthrough", "desc", false);

        // When rendering the row (not selected).
        let line = entry.render_row(false);

        // Then the rendered line does not contain ">".
        let first_span = &line.spans[0];
        assert_eq!(first_span.content, "  ");
    }

    #[test]
    fn render_row_shows_name_and_description() {
        // Given a strategy entry with name and description.
        let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);

        // When rendering the row.
        let line = entry.render_row(false);

        // Then both name and description appear in the rendered text.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("Passthrough"));
        assert!(text.contains("Send as-is"));
    }

    #[test]
    fn render_row_selected_has_background() {
        // Given a strategy entry.
        let entry = make_entry("passthrough", "Passthrough", "desc", false);

        // When rendering the row selected.
        let line = entry.render_row(true);

        // Then the name span has DarkGray background.
        let name_span = &line.spans[1];
        assert_eq!(name_span.style.bg, Some(ratatui::style::Color::DarkGray));
    }

    // --- load_strategy_entries tests ---

    #[test]
    fn load_strategy_entries_returns_all_strategies() {
        // Given test services with the default strategy discovery (4 strategies).
        let services = crate::test_utils::test_services();

        // When loading strategy entries.
        let entries = load_strategy_entries(&services, &PromptStrategyId::passthrough());

        // Then all 4 strategies are returned.
        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn load_strategy_entries_marks_active() {
        // Given test services with the default strategy discovery.
        let services = crate::test_utils::test_services();

        // When loading strategy entries with passthrough as active.
        let entries = load_strategy_entries(&services, &PromptStrategyId::passthrough());

        // Then only the passthrough entry is marked active.
        let active_count = entries.iter().filter(|e| e.is_active).count();
        assert_eq!(active_count, 1);
        let active = entries.iter().find(|e| e.is_active).expect("active entry");
        assert_eq!(active.strategy_id, PromptStrategyId::passthrough());
    }

    #[test]
    fn load_strategy_entries_marks_active_with_non_default() {
        // Given test services with the default strategy discovery.
        let services = crate::test_utils::test_services();

        // When loading strategy entries with sliding_window as active.
        let entries = load_strategy_entries(&services, &PromptStrategyId::sliding_window());

        // Then the sliding_window entry is marked active.
        let active = entries.iter().find(|e| e.is_active).expect("active entry");
        assert_eq!(active.strategy_id, PromptStrategyId::sliding_window());
        assert_eq!(active.name, "Sliding Window");
    }

    // --- sorted_strategy_entries tests ---

    #[test]
    fn sorted_strategy_entries_promotes_active_to_top_when_filter_empty() {
        // Given 4 entries with sliding_window active (not first).
        let entries = vec![
            make_entry("passthrough", "Passthrough", "desc", false),
            make_entry("sliding_window", "Sliding Window", "desc", true),
            make_entry("token_budget", "Token Budget", "desc", false),
            make_entry("compaction", "Compaction", "desc", false),
        ];

        // When sorting with empty filter.
        let result = sorted_strategy_entries(&entries, "");

        // Then sliding_window is promoted to top.
        assert_eq!(result[0].strategy_id, PromptStrategyId::sliding_window());
        assert!(result[0].is_active);
    }

    #[test]
    fn sorted_strategy_entries_preserves_order_when_filtering() {
        // Given entries with sliding_window active.
        let entries = vec![
            make_entry("passthrough", "Passthrough", "desc", false),
            make_entry("sliding_window", "Sliding Window", "desc", true),
        ];

        // When sorting with non-empty filter.
        let result = sorted_strategy_entries(&entries, "search");

        // Then order is unchanged (filter is non-empty).
        assert_eq!(result[0].strategy_id, PromptStrategyId::passthrough());
        assert_eq!(result[1].strategy_id, PromptStrategyId::sliding_window());
    }

    #[test]
    fn sorted_strategy_entries_no_change_when_first_is_active() {
        // Given entries with passthrough active (already first).
        let entries = vec![
            make_entry("passthrough", "Passthrough", "desc", true),
            make_entry("sliding_window", "Sliding Window", "desc", false),
            make_entry("token_budget", "Token Budget", "desc", false),
        ];

        // When sorting with empty filter.
        let result = sorted_strategy_entries(&entries, "");

        // Then order is unchanged.
        assert_eq!(result[0].strategy_id, PromptStrategyId::passthrough());
        assert_eq!(result[1].strategy_id, PromptStrategyId::sliding_window());
        assert_eq!(result[2].strategy_id, PromptStrategyId::token_budget());
    }

    // --- format_strategy_footer tests ---

    #[test]
    fn format_strategy_footer_contains_label_and_name() {
        // Given a strategy name.
        // When formatting the footer.
        let line = format_strategy_footer("Sliding Window");

        // Then "Current:" and the strategy name appear.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("Current: "));
        assert!(text.contains("Sliding Window"));
    }

    #[test]
    fn format_strategy_footer_label_is_dark_gray() {
        // Given a strategy name.
        // When formatting the footer.
        let line = format_strategy_footer("Passthrough");

        // Then the "Current: " span has DarkGray fg.
        let label_span = &line.spans[0];
        assert_eq!(label_span.style.fg, Some(ratatui::style::Color::DarkGray));
    }

    #[test]
    fn format_strategy_footer_name_is_white() {
        // Given a strategy name.
        // When formatting the footer.
        let line = format_strategy_footer("Passthrough");

        // Then the name span has White fg.
        let name_span = &line.spans[1];
        assert_eq!(name_span.style.fg, Some(ratatui::style::Color::White));
    }

    // --- Highlight tests ---

    #[test]
    fn render_row_with_empty_match_indices_same_as_render_row() {
        // Given a strategy entry.
        let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);

        // When rendering with and without match indices.
        let normal = entry.render_row(false);
        let highlighted = entry.render_row_with_highlight(false, &[]);

        // Then the output is identical.
        assert_eq!(normal.spans.len(), highlighted.spans.len());
        for (n, h) in normal.spans.iter().zip(highlighted.spans.iter()) {
            assert_eq!(n.content, h.content);
            assert_eq!(n.style, h.style);
        }
    }

    #[test]
    fn render_row_with_highlight_applies_gray_bg_to_matched_name_chars() {
        // Given a strategy entry with name "Passthrough".
        let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);

        // When highlighting with match at byte 0 (the "P").
        let line = entry.render_row_with_highlight(false, &[0..1]);

        // Then at least one span has gray background.
        let has_highlight = line.spans.iter().any(|s| s.style.bg == Some(ratatui::style::Color::DarkGray));
        assert!(has_highlight, "expected at least one span with gray background");
        // And the highlighted content contains "P".
        let highlighted: String = line
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(ratatui::style::Color::DarkGray))
            .map(|s| s.content.clone())
            .collect();
        assert!(highlighted.contains('P'), "highlighted span should contain 'P'");
    }

    #[test]
    fn render_row_with_highlight_preserves_description() {
        // Given a strategy entry.
        let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);

        // When highlighting with match at byte 0.
        let line = entry.render_row_with_highlight(false, &[0..1]);

        // Then the full text still contains description.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("Send as-is"), "should contain description");
        assert!(text.contains("Passthrough"), "should contain name");
    }
}
