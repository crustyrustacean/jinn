//! Comprehensive tests for tree prefix formatting and entry line assembly.
//!
//! Each test verifies one specific tree rendering behavior using BDD style.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use throbber_widgets_tui::ThrobberState;

use crate::common::app_state::AppState;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::ui::sidebar::sessions::render::entry_line::{
    EntryRenderConfig, assemble_entry_line, tree_prefix,
};
use crate::feat::ui::sidebar::sessions::state::SessionTreeEntry;
use crate::protocol::SessionId;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tree_entry(
    depth: usize,
    ancestor_continuations: Vec<bool>,
    is_last_child: bool,
) -> SessionTreeEntry {
    SessionTreeEntry {
        id: SessionId::new(),
        parent_id: None,
        depth,
        ancestor_continuations,
        is_last_child,
    }
}

fn default_session() -> ChatSessionState {
    ChatSessionState::new()
}

fn default_theme() -> crate::feat::theme::Theme {
    AppState::default().frontend.theme
}

fn make_config(max_title_len: usize) -> EntryRenderConfig<'static> {
    EntryRenderConfig {
        max_title_len,
        theme: Box::leak(Box::new(AppState::default().frontend.theme)),
        throbber_state: Box::leak(Box::new(ThrobberState::default())),
    }
}

// ---------------------------------------------------------------------------
// tree_prefix — root entries
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
// tree_prefix — depth 1
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
// tree_prefix — depth 2
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
// tree_prefix — deep chains
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
// assemble_entry_line — tree prefix inclusion
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn assembled_line_includes_tree_prefix_for_non_root() {
    // Given a child entry at depth 1.
    let tree = tree_entry(1, vec![true], true);
    let session = default_session();
    let config = make_config(30);

    // When assembling the entry line.
    let line = assemble_entry_line(&tree, &session, false, false, &config);

    // Then the line contains the └─ tree characters.
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("└─ "),
        "line should contain └─ , got: {text}"
    );
}

#[rstest::rstest]
fn assembled_line_has_no_tree_prefix_for_root() {
    // Given a root entry.
    let tree = tree_entry(0, vec![], true);
    let session = default_session();
    let config = make_config(30);

    // When assembling the entry line.
    let line = assemble_entry_line(&tree, &session, false, false, &config);

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
    let tree = tree_entry(1, vec![true], false);
    let session = default_session();
    let config = make_config(30);

    // When assembling the entry line.
    let line = assemble_entry_line(&tree, &session, false, false, &config);

    // Then the line has 5 spans: indicator, space, arrow, tree prefix, title.
    assert_eq!(
        line.spans.len(),
        5,
        "child entry should have 5 spans, got {}",
        line.spans.len()
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line — title truncation with tree prefix
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn title_is_truncated_more_at_higher_depth() {
    // Given two entries with the same long title but different depths.
    let long_title = "A very long session title that should be truncated";
    let root_tree = tree_entry(0, vec![], true);
    let child_tree = tree_entry(3, vec![true, true, true], true);

    let mut root_session = default_session();
    root_session.set_title(long_title.to_owned());
    let mut child_session = default_session();
    child_session.set_title(long_title.to_owned());

    let config = make_config(20);

    // When assembling entry lines with limited space.
    let root_line = assemble_entry_line(&root_tree, &root_session, false, false, &config);
    let child_line = assemble_entry_line(&child_tree, &child_session, false, false, &config);

    // Then the root title span is longer than the child title span.
    let root_title = &root_line.spans[3].content;
    let child_title = &child_line.spans[4].content;
    assert!(
        root_title.len() > child_title.len(),
        "root title ({root_title}) should be longer than child title ({child_title})"
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line — active arrow at depth > 0
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn active_arrow_shows_at_depth_greater_than_zero() {
    // Given an active child entry at depth 2.
    let tree = tree_entry(2, vec![true, true], true);
    let session = default_session();
    let config = make_config(30);

    // When assembling the entry line with is_active = true.
    let line = assemble_entry_line(&tree, &session, true, false, &config);

    // Then the arrow span contains ▸.
    let arrow_span = &line.spans[2];
    assert!(
        arrow_span.content.contains('▸'),
        "arrow should contain ▸, got: {}",
        arrow_span.content
    );
}

// ---------------------------------------------------------------------------
// assemble_entry_line — tree prefix color
// ---------------------------------------------------------------------------

#[rstest::rstest]
fn tree_prefix_uses_muted_text_color() {
    // Given a child entry at depth 1.
    let tree = tree_entry(1, vec![true], true);
    let session = default_session();
    let config = make_config(30);

    // When assembling the entry line.
    let line = assemble_entry_line(&tree, &session, false, false, &config);

    // Then the tree prefix span (index 3) uses muted_text color.
    let tree_span = &line.spans[3];
    assert_eq!(
        tree_span.style.fg,
        Some(config.theme.muted_text),
        "tree prefix should use muted_text color"
    );
}
