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
            stats.total_sent += u64::from(record.tokens_sent);
            stats.total_received += u64::from(record.tokens_received);
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

/// Compute aggregated token stats for a session and all its descendants.
///
/// Walks the sessions map recursively: finds all sessions whose
/// `parent_session` points to the target, then recurses into each child.
/// The result includes own stats plus the sum of all descendants.
///
/// This function handles the general case (non-trivial session trees)
/// even though `parent_session` is currently always `None`.
pub fn aggregate_session_stats<S: std::hash::BuildHasher>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
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
fn aggregate_children<S: std::hash::BuildHasher>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
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
