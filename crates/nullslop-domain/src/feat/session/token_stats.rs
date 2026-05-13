//! Token statistics types for session-level tracking.
//!
//! [`TokenRecord`] is an immutable entry in the session's token ledger — one per
//! request/response pair. [`TokenStats`] summarizes totals. [`AggregatedTokenStats`]
//! extends totals to include descendant sessions in the session tree.

use std::collections::HashMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::feat::session::chat_session::ChatSessionState;
use crate::protocol::SessionId;

/// A single immutable record in the session's token ledger.
///
/// Each LLM request/response pair produces one record. The record is created
/// when the request is counted (tokens_sent) and finalized when the response
/// completes (tokens_received). Once both fields are set, the record is never
/// mutated — even if the model or tokenizer changes later.
///
/// Design note: we use a single struct for both phases of the lifecycle.
/// `tokens_received` starts at 0 and is set once when the response completes.
/// This avoids partial-state complexity and keeps the ledger a flat `Vec`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenRecord {
    /// When this request was made.
    pub timestamp: Timestamp,
    /// Tokens sent in the assembled prompt (input).
    pub tokens_sent: u32,
    /// Tokens received in the response (output). 0 until the response completes.
    pub tokens_received: u32,
}

/// Summary statistics derived from a token ledger.
///
/// Computed from `Vec<TokenRecord>` — not stored directly. Use
/// `TokenStats::from_ledger` to derive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenStats {
    /// Total tokens sent across all requests.
    pub total_sent: u64,
    /// Total tokens received across all responses.
    pub total_received: u64,
    /// Number of request/response pairs.
    pub request_count: u64,
}

impl TokenStats {
    /// Derive stats from a token ledger.
    pub fn from_ledger(records: &[TokenRecord]) -> Self {
        let mut stats = Self::default();
        for record in records {
            stats.total_sent += record.tokens_sent as u64;
            stats.total_received += record.tokens_received as u64;
            stats.request_count += 1;
        }
        stats
    }
}

/// Aggregated token statistics for a session and its descendants.
///
/// Used by the status bar to display `↑sent ↓received` with descendant totals.
/// The `ctx:` value comes from the session's cached context size, not from here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregatedTokenStats {
    /// This session's own token stats.
    pub own: TokenStats,
    /// Sum of all descendant sessions' token stats.
    pub children: TokenStats,
}

impl AggregatedTokenStats {
    /// Total tokens sent (own + children).
    pub fn total_sent(&self) -> u64 {
        self.own.total_sent + self.children.total_sent
    }

    /// Total tokens received (own + children).
    pub fn total_received(&self) -> u64 {
        self.own.total_received + self.children.total_received
    }
}

/// Blob key for persisting token stats in the session blobs map.
pub const BLOB_TOKEN_STATS: &str = "token_stats";

/// Blob key for persisting the parent session ID.
pub const BLOB_PARENT_SESSION: &str = "parent_session";

/// Compute aggregated token stats for a session and all its descendants.
///
/// Walks the sessions map recursively: finds all sessions whose
/// `parent_session` points to the target, then recurses into each child.
/// The result includes own stats plus the sum of all descendants.
///
/// This function handles the general case (non-trivial session trees)
/// even though `parent_session` is currently always `None`.
pub fn aggregate_session_stats(
    sessions: &HashMap<SessionId, ChatSessionState>,
    session_id: &SessionId,
) -> AggregatedTokenStats {
    let own_session = sessions.get(session_id);
    let own = own_session
        .map(|s| TokenStats::from_ledger(s.token_ledger()))
        .unwrap_or_default();

    let children = aggregate_children(sessions, session_id);

    AggregatedTokenStats { own, children }
}

/// Recursively sum token stats for all descendants of a session.
fn aggregate_children(
    sessions: &HashMap<SessionId, ChatSessionState>,
    parent_id: &SessionId,
) -> TokenStats {
    let mut total = TokenStats::default();
    for (id, session) in sessions {
        if session.parent_session().as_ref() == Some(parent_id) {
            let child_own = TokenStats::from_ledger(session.token_ledger());
            let child_descendants = aggregate_children(sessions, id);
            total.total_sent += child_own.total_sent + child_descendants.total_sent;
            total.total_received += child_own.total_received + child_descendants.total_received;
            total.request_count += child_own.request_count + child_descendants.request_count;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SessionId;

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
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 100,
            tokens_received: 50,
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
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 50,
            },
            TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 200,
                tokens_received: 75,
            },
            TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 150,
                tokens_received: 60,
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
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 500,
            tokens_received: 250,
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
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 100,
            tokens_received: 50,
        });

        let mut child = ChatSessionState::new();
        child.set_parent_session(parent_id.clone());
        child.push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 200,
            tokens_received: 100,
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
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 1000,
            tokens_received: 500,
        });

        let mut parent = ChatSessionState::new();
        parent.set_parent_session(grandparent_id.clone());
        parent.push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 500,
            tokens_received: 250,
        });

        let mut child = ChatSessionState::new();
        child.set_parent_session(parent_id.clone());
        child.push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 200,
            tokens_received: 100,
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
    fn token_record_round_trips_through_serde() {
        // Given a TokenRecord.
        let record = TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 1234,
            tokens_received: 567,
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&record).expect("serialize");
        let back: TokenRecord = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.tokens_sent, 1234);
        assert_eq!(back.tokens_received, 567);
    }
}
