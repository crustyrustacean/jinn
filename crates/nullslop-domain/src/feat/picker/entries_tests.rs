use crate::feat::theme::default_theme;
use crate::protocol::{PromptStrategyId, StrategyEntry};
use nullslop_selection_widget::PickerItem as _;
use std::ops::Range;

use super::strategy_entries::*;
fn make_entry(id: &str, name: &str, description: &str, is_active: bool) -> StrategyEntry {
    StrategyEntry {
        strategy_id: PromptStrategyId::new(id),
        name: name.to_owned(),
        description: description.to_owned(),
        is_active,
        theme: default_theme(),
    }
}

// --- PickerItem render tests (delegated to protocol impl) ---

#[rstest::rstest]
fn render_row_shows_active_marker_when_active() {
    let entry = make_entry("passthrough", "Passthrough", "desc", true);
    let line = entry.render_row(false);
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains('>'));
}

#[rstest::rstest]
fn render_row_shows_no_marker_when_inactive() {
    let entry = make_entry("passthrough", "Passthrough", "desc", false);
    let line = entry.render_row(false);
    let first_span = &line.spans[0];
    assert_eq!(first_span.content, "  ");
}

#[rstest::rstest]
fn render_row_shows_name_and_description() {
    let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);
    let line = entry.render_row(false);
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains("Passthrough"));
    assert!(text.contains("Send as-is"));
}


// --- load_strategy_entries tests ---

#[rstest::rstest]
fn load_strategy_entries_returns_all_strategies() {
    let services = crate::common::services::Services::new();
    let entries = load_strategy_entries(
        &services,
        &PromptStrategyId::passthrough(),
        &default_theme(),
    );
    assert_eq!(entries.len(), 4);
}

#[rstest::rstest]
fn load_strategy_entries_marks_active() {
    let services = crate::common::services::Services::new();
    let entries = load_strategy_entries(
        &services,
        &PromptStrategyId::passthrough(),
        &default_theme(),
    );
    let active_count = entries.iter().filter(|e| e.is_active).count();
    assert_eq!(active_count, 1);
    let active = entries.iter().find(|e| e.is_active).expect("active entry");
    assert_eq!(active.strategy_id, PromptStrategyId::passthrough());
}

#[rstest::rstest]
fn load_strategy_entries_marks_active_with_non_default() {
    let services = crate::common::services::Services::new();
    let entries = load_strategy_entries(
        &services,
        &PromptStrategyId::sliding_window(),
        &default_theme(),
    );
    let active = entries.iter().find(|e| e.is_active).expect("active entry");
    assert_eq!(active.strategy_id, PromptStrategyId::sliding_window());
    assert_eq!(active.name, "Sliding Window");
}

// --- sorted_strategy_entries tests ---

#[rstest::rstest]
fn sorted_strategy_entries_promotes_active_to_top_when_filter_empty() {
    let entries = vec![
        make_entry("passthrough", "Passthrough", "desc", false),
        make_entry("sliding_window", "Sliding Window", "desc", true),
        make_entry("token_budget", "Token Budget", "desc", false),
        make_entry("compaction", "Compaction", "desc", false),
    ];
    let result = sorted_strategy_entries(&entries, "");
    assert_eq!(result[0].strategy_id, PromptStrategyId::sliding_window());
    assert!(result[0].is_active);
}

#[rstest::rstest]
fn sorted_strategy_entries_preserves_order_when_filtering() {
    let entries = vec![
        make_entry("passthrough", "Passthrough", "desc", false),
        make_entry("sliding_window", "Sliding Window", "desc", true),
    ];
    let result = sorted_strategy_entries(&entries, "search");
    assert_eq!(result[0].strategy_id, PromptStrategyId::passthrough());
    assert_eq!(result[1].strategy_id, PromptStrategyId::sliding_window());
}

#[rstest::rstest]
fn sorted_strategy_entries_no_change_when_first_is_active() {
    let entries = vec![
        make_entry("passthrough", "Passthrough", "desc", true),
        make_entry("sliding_window", "Sliding Window", "desc", false),
        make_entry("token_budget", "Token Budget", "desc", false),
    ];
    let result = sorted_strategy_entries(&entries, "");
    assert_eq!(result[0].strategy_id, PromptStrategyId::passthrough());
    assert_eq!(result[1].strategy_id, PromptStrategyId::sliding_window());
    assert_eq!(result[2].strategy_id, PromptStrategyId::token_budget());
}

// --- format_strategy_footer tests ---

#[rstest::rstest]
fn format_strategy_footer_contains_label_and_name() {
    let line = format_strategy_footer("Sliding Window", &default_theme());
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains("Current: "));
    assert!(text.contains("Sliding Window"));
}


// --- Highlight tests ---

#[rstest::rstest]
fn render_row_with_empty_match_indices_same_as_render_row() {
    let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);
    let normal = entry.render_row(false);
    let highlighted = entry.render_row_with_highlight(false, &[]);
    assert_eq!(normal.spans.len(), highlighted.spans.len());
    for (n, h) in normal.spans.iter().zip(highlighted.spans.iter()) {
        assert_eq!(n.content, h.content);
        assert_eq!(n.style, h.style);
    }
}


#[rstest::rstest]
fn strategy_highlight_row_contains_matched_char() {
    let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);
    #[expect(
        clippy::single_range_in_vec_init,
        reason = "genuinely want a slice containing one Range<usize>"
    )]
    let highlights: &[Range<usize>] = &[0..1];
    let line = entry.render_row_with_highlight(false, highlights);
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(
        text.contains('P'),
        "highlighted row should contain 'P'"
    );
}

#[rstest::rstest]
fn render_row_with_highlight_preserves_description() {
    let entry = make_entry("passthrough", "Passthrough", "Send as-is", false);
    #[expect(
        clippy::single_range_in_vec_init,
        reason = "genuinely want a slice containing one Range<usize>"
    )]
    let highlights: &[Range<usize>] = &[0..1];
    let line = entry.render_row_with_highlight(false, highlights);
    let text: String = line.spans.iter().map(|s| &*s.content).collect();
    assert!(text.contains("Send as-is"), "should contain description");
    assert!(text.contains("Passthrough"), "should contain name");
}
