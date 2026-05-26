//! Tests for tree-wide aggregate statistics.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use std::collections::HashMap;

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::token_stats::TokenRecord;
use crate::feat::session::{aggregate_tree_stats, find_tree_root};
use crate::protocol::{ChatEntry, SessionId};

/// Helper: create an empty session with the given ID.
fn make_session(id: SessionId) -> ChatSessionState {
    let mut state = ChatSessionState::new();
    state.set_session_id(id);
    state
}

/// Helper: create a session with token records and history entries.
fn make_session_with_stats(
    id: SessionId,
    tokens_sent: u32,
    tokens_received: u32,
    cost: Option<f64>,
    turn_count: u32,
) -> ChatSessionState {
    let mut state = ChatSessionState::new();
    state.set_session_id(id);

    if tokens_sent > 0 || tokens_received > 0 {
        state.push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent,
            tokens_received,
            cost,
        });
    }

    // Add user entries to produce the desired turn count.
    for i in 0..turn_count {
        state.push_entry(ChatEntry::user(format!("user msg {i}")));
    }

    state
}

/// Helper: set parent session on a session.
fn set_parent(
    sessions: &mut HashMap<SessionId, ChatSessionState>,
    child: &SessionId,
    parent: &SessionId,
) {
    if let Some(session) = sessions.get_mut(child) {
        session.set_parent_session(parent.clone());
    }
}

// --- find_tree_root tests ---

#[rstest::rstest]
fn single_session_is_own_root() {
    // Given a single session with no parent.
    let id = SessionId::new();
    let mut sessions = HashMap::new();
    sessions.insert(id.clone(), make_session(id.clone()));

    // When finding the root.
    let root = find_tree_root(&sessions, &id);

    // Then it returns the same session.
    assert_eq!(root, id);
}

#[rstest::rstest]
fn child_finds_root() {
    // Given a parent with a child.
    let parent_id = SessionId::new();
    let child_id = SessionId::new();
    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), make_session(parent_id.clone()));
    let mut child = ChatSessionState::new();
    child.set_session_id(child_id.clone());
    child.set_parent_session(parent_id.clone());
    sessions.insert(child_id.clone(), child);

    // When finding root from the child.
    let root = find_tree_root(&sessions, &child_id);

    // Then the root is the parent.
    assert_eq!(root, parent_id);
}

#[rstest::rstest]
fn deeply_nested_finds_root() {
    // Given grandparent → parent → child.
    let gp_id = SessionId::new();
    let parent_id = SessionId::new();
    let child_id = SessionId::new();
    let mut sessions = HashMap::new();
    sessions.insert(gp_id.clone(), make_session(gp_id.clone()));

    let mut parent = ChatSessionState::new();
    parent.set_session_id(parent_id.clone());
    parent.set_parent_session(gp_id.clone());
    sessions.insert(parent_id.clone(), parent);

    let mut child = ChatSessionState::new();
    child.set_session_id(child_id.clone());
    child.set_parent_session(parent_id.clone());
    sessions.insert(child_id.clone(), child);

    // When finding root from the child.
    let root = find_tree_root(&sessions, &child_id);

    // Then the root is the grandparent.
    assert_eq!(root, gp_id);
}

#[rstest::rstest]
fn orphan_session_treated_as_root() {
    // Given a session whose parent_session points to a non-existent ID.
    let orphan_id = SessionId::new();
    let ghost_parent = SessionId::new();
    let mut orphan = ChatSessionState::new();
    orphan.set_session_id(orphan_id.clone());
    orphan.set_parent_session(ghost_parent);

    let mut sessions = HashMap::new();
    sessions.insert(orphan_id.clone(), orphan);

    // When finding root.
    let root = find_tree_root(&sessions, &orphan_id);

    // Then the orphan itself is the root (ghost parent not in map).
    assert_eq!(root, orphan_id);
}

// --- aggregate_tree_stats tests ---

#[rstest::rstest]
fn single_session_returns_own_stats() {
    // Given a single session with known stats.
    let id = SessionId::new();
    let mut sessions = HashMap::new();
    sessions.insert(id.clone(), make_session_with_stats(id.clone(), 100, 50, Some(0.01), 2));

    // When aggregating tree stats.
    let stats = aggregate_tree_stats(&sessions, &id);

    // Then it returns the session's own stats.
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.total_sent, 100);
    assert_eq!(stats.total_received, 50);
    assert!(stats.total_cost - 0.01 < 1e-10);
    assert_eq!(stats.total_turns, 2); // 2 user entries
}

#[rstest::rstest]
fn parent_with_children_sums_all() {
    // Given a parent with 2 children.
    let parent_id = SessionId::new();
    let child1_id = SessionId::new();
    let child2_id = SessionId::new();

    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), make_session_with_stats(parent_id.clone(), 100, 50, Some(0.01), 2));
    sessions.insert(child1_id.clone(), make_session_with_stats(child1_id.clone(), 200, 100, Some(0.02), 1));
    sessions.insert(child2_id.clone(), make_session_with_stats(child2_id.clone(), 300, 150, Some(0.03), 3));

    set_parent(&mut sessions, &child1_id, &parent_id);
    set_parent(&mut sessions, &child2_id, &parent_id);

    // When aggregating from parent.
    let stats = aggregate_tree_stats(&sessions, &parent_id);

    // Then all sessions are summed.
    assert_eq!(stats.session_count, 3);
    assert_eq!(stats.total_sent, 600);
    assert_eq!(stats.total_received, 300);
    assert!(stats.total_cost - 0.06 < 1e-10);
    assert_eq!(stats.total_turns, 6); // 2 + 1 + 3
}

#[rstest::rstest]
fn child_sees_entire_tree() {
    // Given a parent with 2 children.
    let parent_id = SessionId::new();
    let child1_id = SessionId::new();
    let child2_id = SessionId::new();

    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), make_session_with_stats(parent_id.clone(), 100, 50, None, 1));
    sessions.insert(child1_id.clone(), make_session_with_stats(child1_id.clone(), 200, 100, None, 2));
    sessions.insert(child2_id.clone(), make_session_with_stats(child2_id.clone(), 300, 150, None, 3));

    set_parent(&mut sessions, &child1_id, &parent_id);
    set_parent(&mut sessions, &child2_id, &parent_id);

    // When aggregating from child1.
    let stats = aggregate_tree_stats(&sessions, &child1_id);

    // Then the result includes parent + both children.
    assert_eq!(stats.session_count, 3);
    assert_eq!(stats.total_sent, 600);
    assert_eq!(stats.total_received, 300);
    assert_eq!(stats.total_turns, 6); // 1 + 2 + 3
}

#[rstest::rstest]
fn deeply_nested_tree_sums_all() {
    // Given grandparent → parent → child.
    let gp_id = SessionId::new();
    let parent_id = SessionId::new();
    let child_id = SessionId::new();

    let mut sessions = HashMap::new();
    sessions.insert(gp_id.clone(), make_session_with_stats(gp_id.clone(), 10, 5, None, 1));
    sessions.insert(parent_id.clone(), make_session_with_stats(parent_id.clone(), 20, 10, None, 1));
    sessions.insert(child_id.clone(), make_session_with_stats(child_id.clone(), 30, 15, None, 1));

    set_parent(&mut sessions, &parent_id, &gp_id);
    set_parent(&mut sessions, &child_id, &parent_id);

    // When aggregating from the child.
    let stats = aggregate_tree_stats(&sessions, &child_id);

    // Then all 3 sessions are included.
    assert_eq!(stats.session_count, 3);
    assert_eq!(stats.total_sent, 60);
    assert_eq!(stats.total_received, 30);
    assert_eq!(stats.total_turns, 3);
}

#[rstest::rstest]
fn disconnected_sessions_excluded() {
    // Given a parent with a child and a disconnected session.
    let parent_id = SessionId::new();
    let child_id = SessionId::new();
    let disconnected_id = SessionId::new();

    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), make_session_with_stats(parent_id.clone(), 100, 50, None, 1));
    sessions.insert(child_id.clone(), make_session_with_stats(child_id.clone(), 200, 100, None, 1));
    sessions.insert(disconnected_id.clone(), make_session_with_stats(disconnected_id.clone(), 999, 999, None, 99));

    set_parent(&mut sessions, &child_id, &parent_id);

    // When aggregating from parent.
    let stats = aggregate_tree_stats(&sessions, &parent_id);

    // Then disconnected is excluded.
    assert_eq!(stats.session_count, 2);
    assert_eq!(stats.total_sent, 300);
    assert_eq!(stats.total_received, 150);
    assert_eq!(stats.total_turns, 2);
}

#[rstest::rstest]
fn empty_sessions_produce_zeros() {
    // Given sessions with no token records or history.
    let parent_id = SessionId::new();
    let child_id = SessionId::new();

    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), make_session(parent_id.clone()));
    let mut child = ChatSessionState::new();
    child.set_session_id(child_id.clone());
    child.set_parent_session(parent_id.clone());
    sessions.insert(child_id.clone(), child);

    // When aggregating.
    let stats = aggregate_tree_stats(&sessions, &parent_id);

    // Then all values are zero.
    assert_eq!(stats.session_count, 2);
    assert_eq!(stats.total_sent, 0);
    assert_eq!(stats.total_received, 0);
    assert!(stats.total_cost < f64::EPSILON);
    assert_eq!(stats.total_turns, 0);
}
