//! Comprehensive tests for tree prefix formatting and entry line assembly.
//!
//! Each test verifies one specific tree rendering behavior using BDD style.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use jiff::Timestamp;
use throbber_widgets_tui::ThrobberState;

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::render::entry_line::{assemble_entry_line, tree_prefix};
use crate::feat::ui::sidebar::sessions::state::{SessionEntry, SessionEntryKind};
use crate::protocol::SessionId;

use unicode_segmentation::UnicodeSegmentation;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tree_entry(
    depth: usize,
    ancestor_continuations: Vec<bool>,
    is_last_child: bool,
) -> SessionEntry {
    SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Test".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth,
        ancestor_continuations,
        is_last_child,
    }
}

fn default_theme() -> crate::feat::theme::Theme {
    AppState::default().frontend.theme
}

fn idle_throbber() -> ThrobberState {
    ThrobberState::default()
}

// ---------------------------------------------------------------------------
// tree_prefix - root entries
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_is_empty_for_root_entry() {
    // Given a root entry (depth 0).
    let entry = tree_entry(0, vec![], true);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is empty.
    assert_eq!(prefix, "");
}

// ---------------------------------------------------------------------------
// tree_prefix - depth 1
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_last_child_at_depth_1() {
    // Given a last child at depth 1.
    let entry = tree_entry(1, vec![true], true);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is └─ (root continuation skipped).
    assert_eq!(prefix, "└─ ");
}

#[rstest::rstest]
fn tree_prefix_not_last_child_at_depth_1() {
    // Given a non-last child at depth 1.
    let entry = tree_entry(1, vec![true], false);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is ├─ (root continuation skipped).
    assert_eq!(prefix, "├─ ");
}

// ---------------------------------------------------------------------------
// tree_prefix - depth 2
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_last_child_with_continuing_ancestor() {
    // Given a last child at depth 2 with a continuing intermediate ancestor.
    let entry = tree_entry(2, vec![true, true], true);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is │  └─ (root skipped, intermediate continues).
    assert_eq!(prefix, "│  └─ ");
}

#[rstest::rstest]
fn tree_prefix_last_child_with_non_continuing_ancestor() {
    // Given a last child at depth 2 with a non-continuing intermediate ancestor.
    let entry = tree_entry(2, vec![true, false], true);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is    └─ (root skipped, intermediate is space).
    assert_eq!(prefix, "   └─ ");
}

#[rstest::rstest]
fn tree_prefix_not_last_child_with_continuing_ancestor() {
    // Given a non-last child at depth 2 with a continuing intermediate ancestor.
    let entry = tree_entry(2, vec![true, true], false);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then the prefix is │  ├─ (root skipped, intermediate continues).
    assert_eq!(prefix, "│  ├─ ");
}

// ---------------------------------------------------------------------------
// tree_prefix - deep chains
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_deep_chain_not_last() {
    // Given a non-last child at depth 5 with mixed continuations.
    let entry = tree_entry(5, vec![true, true, false, true, true], false);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then continuation chars are correct: │     │  │  ├─ (root skipped, 4 intermediate segments).
    assert_eq!(prefix, "│     │  │  ├─ ");
}

#[rstest::rstest]
fn tree_prefix_deep_chain_last() {
    // Given a last child at depth 5 with mixed continuations.
    let entry = tree_entry(5, vec![true, true, false, true, true], true);

    // When computing tree prefix.
    let prefix = tree_prefix(&entry);

    // Then continuation chars are correct: │     │  │  └─ (root skipped, 4 intermediate segments).
    assert_eq!(prefix, "│     │  │  └─ ");
}

// ---------------------------------------------------------------------------
// assemble_entry_line - tree prefix inclusion
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn assembled_line_includes_tree_prefix_for_non_root() {
    // Given a child entry at depth 1.
    let entry = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Child".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 1,
        ancestor_continuations: vec![true],
        is_last_child: true,
    };
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);

    // Then the line contains the \u{2514}\u{2500} tree characters.
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("\u{2514}\u{2500} "),
        "line should contain \u{2514}\u{2500} , got: {text}"
    );
}

#[rstest::rstest]
fn assembled_line_has_no_tree_prefix_for_root() {
    // Given a root entry.
    let entry = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Root".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 0,
        ancestor_continuations: vec![],
        is_last_child: true,
    };
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);

    // Then the line has 4 spans: indicator, space, arrow, title (no tree prefix span).
    assert_eq!(
        line.spans.len(),
        4,
        "root entry should have 4 spans, got {}",
        line.spans.len()
    );
}

#[rstest::rstest]
fn assembled_line_has_tree_prefix_span_for_child() {
    // Given a child entry at depth 1.
    let entry = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Child".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 1,
        ancestor_continuations: vec![true],
        is_last_child: false,
    };
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);

    // Then the line has 5 spans: indicator, space, arrow, tree prefix, title.
    assert_eq!(
        line.spans.len(),
        5,
        "child entry should have 5 spans, got {}",
        line.spans.len()
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line - title truncation with tree prefix
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn title_is_truncated_more_at_higher_depth() {
    // Given two entries with the same title but different depths.
    let root = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "A very long session title that should be truncated".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 0,
        ancestor_continuations: vec![],
        is_last_child: true,
    };
    let child = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "A very long session title that should be truncated".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 3,
        ancestor_continuations: vec![true, true, true],
        is_last_child: true,
    };
    let theme = default_theme();
    let max_len = 20;

    // When assembling entry lines with limited space.
    let root_line = assemble_entry_line(&root, false, max_len, &idle_throbber(), &theme);
    let child_line = assemble_entry_line(&child, false, max_len, &idle_throbber(), &theme);

    // Then the root title span is longer than the child title span.
    // Root has 4 spans: indicator, space, arrow, title.
    // Child has 5 spans: indicator, space, arrow, tree prefix, title.
    let root_title = &root_line.spans[3].content;
    let child_title = &child_line.spans[4].content;
    assert!(
        root_title.len() > child_title.len(),
        "root title ({root_title}) should be longer than child title ({child_title})"
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line - active arrow at depth > 0
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn active_arrow_shows_at_depth_greater_than_zero() {
    // Given an active child entry at depth 2.
    let entry = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Deep Child".to_owned(),
        is_active: true,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 2,
        ancestor_continuations: vec![true, true],
        is_last_child: true,
    };
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);

    // Then the arrow span contains ▸.
    let arrow_span = &line.spans[2];
    assert!(
        arrow_span.content.contains('▸'),
        "arrow should contain ▸, got: {}",
        arrow_span.content
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line - tree prefix color
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_uses_muted_text_color() {
    // Given a child entry at depth 1.
    let entry = SessionEntry {
        kind: SessionEntryKind::Session,
        id: SessionId::new(),
        title: "Child".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 1,
        ancestor_continuations: vec![true],
        is_last_child: true,
    };
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);

    // Then the tree prefix span (index 3) uses muted_text color.
    let tree_span = &line.spans[3];
    assert_eq!(
        tree_span.style.fg,
        Some(theme.muted_text),
        "tree prefix should use muted_text color"
    );
}

// ---------------------------------------------------------------------------

#[test]
fn non_judge_child_at_depth_1_uses_grapheme_count_for_tree() {
    // Given a non-judge child at depth 1 with a long title.
    // Tree prefix "├─ " = 3 graphemes (but 7 bytes).
    let mut entry = tree_entry(1, vec![true], false);
    entry.title = "ABCDEFGHIJ".to_owned();
    let theme = default_theme();
    // max_title_len = 7 → budget = 7 - 3 (tree prefix graphemes) = 4 graphemes for title.
    // Title "ABCDEFGHIJ" truncated to 3 + "…" = 4 graphemes.
    let max_title_len = 7;

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, max_title_len, &idle_throbber(), &theme);

    // Then the title span has exactly 4 graphemes.
    let title_span = &line.spans[4].content;
    let grapheme_count = title_span.graphemes(true).count();
    assert_eq!(
        grapheme_count, 4,
        "title span should have 4 graphemes, got {grapheme_count}: {title_span}"
    );
    assert!(
        title_span.ends_with('…'),
        "truncated title should end with ellipsis, got: {title_span}"
    );
}

// ---------------------------------------------------------------------------
// Plugin entry rendering
// ---------------------------------------------------------------------------

fn plugin_entry(enabled: bool) -> SessionEntry {
    SessionEntry {
        kind: SessionEntryKind::Plugin { enabled },
        id: SessionId::new(),
        title: "my-plugin".to_owned(),
        is_active: false,
        created_at: Timestamp::now(),
        is_idle: true,
        last_entry_is_error: false,
        parent_id: None,
        depth: 1,
        ancestor_continuations: vec![false],
        is_last_child: true,
    }
}

#[test]
fn render_plugin_entry_shows_plugin_name() {
    // Given a plugin entry with an enabled plugin.
    let entry = plugin_entry(true);
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);
    let text = line.to_string();

    // Then the plugin name appears in the output.
    assert!(
        text.contains("my-plugin"),
        "expected plugin name in rendered line, got: {text}"
    );
}

#[test]
fn render_plugin_entry_shows_lightning_icon() {
    // Given a plugin entry.
    let entry = plugin_entry(true);
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);
    let text = line.to_string();

    // Then the lightning bolt icon appears in the output.
    assert!(
        text.contains('\u{26A1}'),
        "expected lightning bolt icon in rendered line, got: {text}"
    );
}

#[test]
fn render_plugin_entry_dimmed_when_disabled() {
    // Given a disabled plugin entry.
    let entry = plugin_entry(false);
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);
    let text = line.to_string();

    // Then the disabled marker appears.
    assert!(
        text.contains('\u{2717}'),
        "expected disabled marker in rendered line, got: {text}"
    );
}

#[test]
fn render_plugin_entry_enabled_not_dimmed() {
    // Given an enabled plugin entry.
    let entry = plugin_entry(true);
    let theme = default_theme();

    // When assembling the entry line.
    let line = assemble_entry_line(&entry, false, 30, &idle_throbber(), &theme);
    let text = line.to_string();

    // Then no disabled marker appears.
    assert!(
        !text.contains('\u{2717}'),
        "expected no disabled marker in enabled entry, got: {text}"
    );
}
