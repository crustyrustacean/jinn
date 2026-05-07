use nullslop_selection_widget::PickerItem as _;
use std::ops::Range;

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
    #[expect(clippy::single_range_in_vec_init, reason = "genuinely want a slice containing one Range<usize>")]
    let highlights: &[Range<usize>] = &[0..1];
    let line = entry.render_row_with_highlight(false, highlights);

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
    #[expect(clippy::single_range_in_vec_init, reason = "genuinely want a slice containing one Range<usize>")]
    let highlights: &[Range<usize>] = &[0..1];
    let line = entry.render_row_with_highlight(false, highlights);

    // Then the full text still contains description.
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains("Send as-is"), "should contain description");
    assert!(text.contains("Passthrough"), "should contain name");
}
