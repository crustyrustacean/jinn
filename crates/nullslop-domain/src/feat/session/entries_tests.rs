//! Tests for tree-aware session picker sorting.
//!
//! Covers the `sort_entries_tree_aware` function which sorts sessions
//! so that whole trees move as a unit, positioned by the most recent
//! `updated_at` across all nodes in the tree.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::feat::session::chat_session::SessionState;
use crate::feat::session::entries::sort_entries_tree_aware;
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::theme::default_theme;
use crate::protocol::SessionId;

/// Helper to create a timestamp from a Unix epoch second offset.
fn ts(offset_secs: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(offset_secs).expect("valid timestamp")
}

/// Helper to create a root SessionTreeEntry.
fn root(id: &str, title: &str, updated_at: jiff::Timestamp, state: SessionState) -> SessionTreeEntry {
    SessionTreeEntry::new(
        SessionId::from(id.to_owned()),
        title.to_owned(),
        updated_at,
        default_theme(),
        state,
        None,
    )
}

/// Helper to create a child SessionTreeEntry.
fn child(
    id: &str,
    parent: &str,
    title: &str,
    updated_at: jiff::Timestamp,
    state: SessionState,
) -> SessionTreeEntry {
    SessionTreeEntry::new(
        SessionId::from(id.to_owned()),
        title.to_owned(),
        updated_at,
        default_theme(),
        state,
        Some(SessionId::from(parent.to_owned())),
    )
}

#[rstest::rstest]
fn single_root_no_children_stays_in_output() {
    // Given a single entry with no children.
    let mut entries = vec![root("a", "Alpha", ts(100), SessionState::Loaded)];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then the single entry is present.
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
}

#[rstest::rstest]
fn root_with_recent_child_appears_before_standalone_root() {
    // Given root A at T1 with child C at T5, and standalone root B at T3.
    let mut entries = vec![
        root("a", "Alpha", ts(100), SessionState::Loaded),
        root("b", "Beta", ts(300), SessionState::Loaded),
        child("c", "a", "Charlie", ts(500), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then tree A (max T5) appears before standalone B (T3).
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("c".to_owned()));
    assert_eq!(entries[2].session_id, SessionId::from("b".to_owned()));
}

#[rstest::rstest]
fn older_root_with_recent_child_beats_newer_root_alone() {
    // Given tree A: root at T1, child at T10. Tree B: root at T5, no children.
    let mut entries = vec![
        root("a", "Alpha", ts(100), SessionState::Loaded),
        child("c", "a", "Child", ts(1000), SessionState::Loaded),
        root("b", "Beta", ts(500), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then tree A (max T10) appears before tree B (T5).
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("c".to_owned()));
    assert_eq!(entries[2].session_id, SessionId::from("b".to_owned()));
}

#[rstest::rstest]
fn loaded_tree_appears_above_archived_tree_regardless_of_timestamps() {
    // Given an Archived root at T100 and a Loaded root at T1.
    let mut entries = vec![
        root("arch", "Archived", ts(10000), SessionState::Archived),
        root("load", "Loaded", ts(100), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then the Loaded tree appears first.
    assert_eq!(entries[0].session_id, SessionId::from("load".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("arch".to_owned()));
}

#[rstest::rstest]
fn multi_level_tree_sorts_by_max_across_entire_subtree() {
    // Given root A at T1, child C at T3, grandchild G at T10.
    // And standalone root B at T5.
    let mut entries = vec![
        root("a", "Alpha", ts(100), SessionState::Loaded),
        child("c", "a", "Child", ts(300), SessionState::Loaded),
        child("g", "c", "Grandchild", ts(1000), SessionState::Loaded),
        root("b", "Beta", ts(500), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then tree A (max T10 via grandchild) appears before B (T5).
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("c".to_owned()));
    assert_eq!(entries[2].session_id, SessionId::from("g".to_owned()));
    assert_eq!(entries[3].session_id, SessionId::from("b".to_owned()));
}

#[rstest::rstest]
fn children_within_tree_sorted_by_updated_at_descending() {
    // Given root A with two children: C1 at T3 and C2 at T7.
    let mut entries = vec![
        root("a", "Alpha", ts(100), SessionState::Loaded),
        child("c1", "a", "Child1", ts(300), SessionState::Loaded),
        child("c2", "a", "Child2", ts(700), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then C2 (T7) appears before C1 (T3).
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("c2".to_owned()));
    assert_eq!(entries[2].session_id, SessionId::from("c1".to_owned()));
}

#[rstest::rstest]
fn orphaned_entry_treated_as_root() {
    // Given entry O with parent_id pointing to non-existent session X.
    // And a normal root A at T5.
    let mut entries = vec![
        child("o", "nonexistent", "Orphan", ts(100), SessionState::Loaded),
        root("a", "Alpha", ts(500), SessionState::Loaded),
    ];

    // When sorting.
    sort_entries_tree_aware(&mut entries);

    // Then A (T5) appears before O (T1), and O is treated as a root.
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].session_id, SessionId::from("a".to_owned()));
    assert_eq!(entries[1].session_id, SessionId::from("o".to_owned()));
}
