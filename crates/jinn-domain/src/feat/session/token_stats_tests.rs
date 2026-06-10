#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::token_stats::{
    AggregatedTokenStats, TokenRecord, TokenStats, aggregate_session_stats,
};
use crate::protocol::SessionId;
use std::collections::HashMap;

// --- TokenStats::from_ledger ---

#[rstest::rstest]
fn from_ledger_returns_defaults_for_empty() {
    // Given an empty ledger.
    let stats = TokenStats::from_ledger(&[]);

    // Then all fields are zero.
    assert_eq!(stats.total_sent, 0);
    assert_eq!(stats.total_received, 0);
    assert_eq!(stats.request_count, 0);
}

#[rstest::rstest]
fn from_ledger_sums_single_record() {
    // Given a ledger with one record.
    let records = vec![TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 100,
        tokens_received: 50,
        cost: None,
    }];

    // When deriving stats.
    let stats = TokenStats::from_ledger(&records);

    // Then totals match.
    assert_eq!(stats.total_sent, 100);
    assert_eq!(stats.total_received, 50);
    assert_eq!(stats.request_count, 1);
}

#[rstest::rstest]
fn from_ledger_sums_multiple_records() {
    // Given a ledger with three records.
    let records = vec![
        TokenRecord {
            model_used: None,
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 100,
            tokens_received: 50,
            cost: None,
        },
        TokenRecord {
            model_used: None,
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 200,
            tokens_received: 75,
            cost: None,
        },
        TokenRecord {
            model_used: None,
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 150,
            tokens_received: 60,
            cost: None,
        },
    ];

    // When deriving stats.
    let stats = TokenStats::from_ledger(&records);

    // Then totals are summed.
    assert_eq!(stats.total_sent, 450);
    assert_eq!(stats.total_received, 185);
    assert_eq!(stats.request_count, 3);
}

// --- AggregatedTokenStats ---

#[rstest::rstest]
fn aggregated_totals_sum_own_and_children() {
    // Given aggregated stats with own and children.
    let agg = AggregatedTokenStats {
        own: TokenStats {
            total_sent: 100,
            total_received: 50,
            request_count: 1,
        },
        children: TokenStats {
            total_sent: 200,
            total_received: 100,
            request_count: 2,
        },
        own_cost: 0.01,
        children_cost: 0.02,
    };

    // Then totals sum both.
    assert_eq!(agg.total_sent(), 300);
    assert_eq!(agg.total_received(), 150);
}

// --- aggregate_session_stats ---

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
    });

    let mut child = ChatSessionState::new();
    child.set_parent_session(parent_id.clone());
    child.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 200,
        tokens_received: 100,
        cost: None,
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
    });

    let mut parent = ChatSessionState::new();
    parent.set_parent_session(grandparent_id.clone());
    parent.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 500,
        tokens_received: 250,
        cost: None,
    });

    let mut child = ChatSessionState::new();
    child.set_parent_session(parent_id.clone());
    child.push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 200,
        tokens_received: 100,
        cost: None,
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

// --- TokenRecord serde ---


#[rstest::rstest]
fn token_record_with_model_used_round_trips_through_serde() {
    // Given a record with model_used set.
    let record = TokenRecord {
        model_used: Some("ollama/llama3".to_owned()),
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 100,
        tokens_received: 50,
        cost: Some(0.003),
    };

    // When round-tripping through JSON.
    let json = serde_json::to_string(&record).expect("serialize");
    let deserialized: TokenRecord = serde_json::from_str(&json).expect("deserialize");

    // Then model_used is preserved.
    assert_eq!(deserialized.model_used.as_deref(), Some("ollama/llama3"));
}

#[rstest::rstest]
fn token_record_without_model_used_deserializes_as_none() {
    // Given a JSON record without model_used (legacy format).
    let json = serde_json::json!({
        "timestamp": "2024-01-01T00:00:00Z",
        "tokens_sent": 100,
        "tokens_received": 50,
        "cost": null
    })
    .to_string();

    // When deserializing.
    let record: TokenRecord = serde_json::from_str(&json).expect("deserialize");

    // Then model_used is None (backward compat).
    assert!(record.model_used.is_none());
}
