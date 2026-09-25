#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::feat::session::aggregate_session_stats;
use jinn_core_types::SessionId;
use jinn_session_state::ChatSessionState;
use jinn_token_count_msg::TokenRecord;
use std::collections::HashMap;

#[rstest::rstest]
fn aggregate_for_unknown_session_returns_defaults() {
    // Given an empty sessions map.
    let sessions = HashMap::new();
    let session_id = SessionId::new();

    // When aggregating for a non-existent session.
    let stats = aggregate_session_stats(&sessions, &session_id);

    // Then own stats are default.
    assert_eq!(stats.own.total_sent, 0);
    assert_eq!(stats.own.total_received, 0);
    assert_eq!(stats.children.total_sent, 0);
}

#[rstest::rstest]
fn aggregate_returns_own_stats_for_session_with_no_children() {
    // Given a single session with token records.
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 500,
        tokens_received: 250,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut sessions = HashMap::new();
    sessions.insert(session_id.clone(), session);

    // When aggregating.
    let stats = aggregate_session_stats(&sessions, &session_id);

    // Then own stats reflect the ledger.
    assert_eq!(stats.own.total_sent, 500);
    assert_eq!(stats.own.total_received, 250);
    assert_eq!(stats.children.total_sent, 0);
}

#[rstest::rstest]
fn aggregate_includes_child_session_stats() {
    // Given a parent and child session.
    let parent_id = SessionId::new();
    let child_id = SessionId::new();

    let mut parent = ChatSessionState::new();
    parent.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 100,
        tokens_received: 50,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut child = ChatSessionState::new();
    child.set_parent_session(parent_id.clone());
    child.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 200,
        tokens_received: 100,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut sessions = HashMap::new();
    sessions.insert(parent_id.clone(), parent);
    sessions.insert(child_id, child);

    // When aggregating for the parent.
    let stats = aggregate_session_stats(&sessions, &parent_id);

    // Then own stats are the parent's.
    assert_eq!(stats.own.total_sent, 100);
    assert_eq!(stats.own.total_received, 50);
    // And children stats include the child.
    assert_eq!(stats.children.total_sent, 200);
    assert_eq!(stats.children.total_received, 100);
    // And totals sum both.
    assert_eq!(stats.total_sent(), 300);
    assert_eq!(stats.total_received(), 150);
}

#[rstest::rstest]
fn aggregate_handles_nested_children() {
    // Given grandparent → parent → child.
    let grandparent_id = SessionId::new();
    let parent_id = SessionId::new();
    let child_id = SessionId::new();

    let mut grandparent = ChatSessionState::new();
    grandparent.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1000,
        tokens_received: 500,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut parent = ChatSessionState::new();
    parent.set_parent_session(grandparent_id.clone());
    parent.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 500,
        tokens_received: 250,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut child = ChatSessionState::new();
    child.set_parent_session(parent_id.clone());
    child.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 200,
        tokens_received: 100,
        cost: None,
        prompt_tokens: None,
        cached_tokens: None,
    });

    let mut sessions = HashMap::new();
    sessions.insert(grandparent_id.clone(), grandparent);
    sessions.insert(parent_id, parent);
    sessions.insert(child_id, child);

    // When aggregating for the grandparent.
    let stats = aggregate_session_stats(&sessions, &grandparent_id);

    // Then totals include all descendants recursively.
    assert_eq!(stats.own.total_sent, 1000);
    assert_eq!(stats.children.total_sent, 700); // parent 500 + child 200
    assert_eq!(stats.total_sent(), 1700);
    assert_eq!(stats.total_received(), 850);
}
