//! Token statistics types for session-level tracking.
//!
//! [`TokenRecord`] is an immutable entry in the session's token ledger - one per
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
/// mutated - even if the model or tokenizer changes later.
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
    /// Cost in USD reported by the provider for this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// The concrete model ID that handled this request.
    /// For alloys, this is the resolved model; for single models, the model itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_used: Option<String>,
    /// Provider-reported prompt token count for this request.
    ///
    /// Set when the stream completed with usage data; `None` when the
    /// provider did not report it (e.g. a cancelled turn). The pre-send
    /// local estimate (`tokens_sent`) is never overwritten and is used
    /// as a fallback when this is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    /// Provider-reported count of prompt tokens served from a cache
    /// (`usage.prompt_tokens_details.cached_tokens`).
    ///
    /// OpenAI-compat only (e.g. OpenRouter); `None` for providers that do
    /// not report cache details or for cancelled turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

/// Summary statistics derived from a token ledger.
///
/// Computed from `Vec<TokenRecord>` - not stored directly. Use
/// `TokenStats::from_ledger` to derive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenStats {
    /// Total tokens sent across all requests (raw local estimate sum).
    pub total_sent: u64,
    /// Total tokens received across all responses.
    pub total_received: u64,
    /// Number of request/response pairs.
    pub request_count: u64,
    /// Effective sent total: provider-reported `prompt_tokens` when present,
    /// else the local estimate `tokens_sent`. Used for the `↑sent` display.
    pub effective_sent: u64,
    /// Sum of provider-reported `prompt_tokens` over measured turns only
    /// (records where it is `Some`). The denominator for the cache ratio.
    pub measured_sent: u64,
    /// Sum of provider-reported cache-hit counts (`cached_tokens`).
    pub cached_total: u64,
}

impl TokenStats {
    /// Derive stats from a token ledger.
    pub fn from_ledger(records: &[TokenRecord]) -> Self {
        let mut stats = Self::default();
        for record in records {
            stats.total_sent += u64::from(record.tokens_sent);
            stats.total_received += u64::from(record.tokens_received);
            stats.request_count += 1;
            stats.effective_sent += u64::from(record.prompt_tokens.unwrap_or(record.tokens_sent));
            if let Some(prompt) = record.prompt_tokens {
                stats.measured_sent += u64::from(prompt);
            }
            stats.cached_total += u64::from(record.cached_tokens.unwrap_or(0));
        }
        stats
    }

    /// Sum of all costs in the ledger.
    pub fn total_cost(records: &[TokenRecord]) -> f64 {
        records.iter().filter_map(|r| r.cost).sum()
    }
}

/// Aggregated token statistics for a session and its descendants.
///
/// Used by the status bar to display `↑sent ↓received` with descendant totals.
/// The `ctx:` value comes from the session's cached context size, not from here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AggregatedTokenStats {
    /// This session's own token stats.
    pub own: TokenStats,
    /// Sum of all descendant sessions' token stats.
    pub children: TokenStats,
    /// Total cost from this session's own records.
    pub own_cost: f64,
    /// Total cost from all descendant sessions.
    pub children_cost: f64,
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

    /// Total cost across own and children sessions.
    pub fn total_cost(&self) -> f64 {
        self.own_cost + self.children_cost
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
pub fn aggregate_session_stats<S>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    session_id: &SessionId,
) -> AggregatedTokenStats
where
    S: std::hash::BuildHasher,
{
    let own_session = sessions.get(session_id);
    let own = own_session
        .map(|s| TokenStats::from_ledger(s.token_ledger()))
        .unwrap_or_default();

    let children = aggregate_children(sessions, session_id);

    let own_cost = own_session.map_or(0.0, |s| TokenStats::total_cost(s.token_ledger()));
    let children_cost = aggregate_children_cost(sessions, session_id);

    AggregatedTokenStats {
        own,
        children,
        own_cost,
        children_cost,
    }
}

/// Recursively sum token stats for all descendants of a session.
fn aggregate_children<S>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    parent_id: &SessionId,
) -> TokenStats
where
    S: std::hash::BuildHasher,
{
    let mut total = TokenStats::default();
    for (id, session) in sessions {
        if session.parent_session().as_ref() == Some(parent_id) {
            let child_own = TokenStats::from_ledger(session.token_ledger());
            let child_descendants = aggregate_children(sessions, id);
            total.total_sent += child_own.total_sent + child_descendants.total_sent;
            total.total_received += child_own.total_received + child_descendants.total_received;
            total.request_count += child_own.request_count + child_descendants.request_count;
            total.effective_sent += child_own.effective_sent + child_descendants.effective_sent;
            total.measured_sent += child_own.measured_sent + child_descendants.measured_sent;
            total.cached_total += child_own.cached_total + child_descendants.cached_total;
        }
    }
    total
}

/// Recursively sum costs for all descendants of a session.
fn aggregate_children_cost<S>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    parent_id: &SessionId,
) -> f64
where
    S: std::hash::BuildHasher,
{
    let mut total = 0.0;
    for (id, session) in sessions {
        if session.parent_session().as_ref() == Some(parent_id) {
            let own = TokenStats::total_cost(session.token_ledger());
            let descendants = aggregate_children_cost(sessions, id);
            total += own + descendants;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::float_cmp,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::session::chat_session::ChatSessionState;
    use jiff::Timestamp;

    /// Build a sessions map with a parent→child→grandchild tree.
    /// Each session has one token record with known values.
    fn build_tree() -> HashMap<SessionId, ChatSessionState> {
        let mut root = ChatSessionState::new();
        root.push_token_record(TokenRecord {
            model_used: None,
            timestamp: Timestamp::now(),
            tokens_sent: 100,
            tokens_received: 200,
            cost: Some(0.01),
            prompt_tokens: None,
            cached_tokens: None,
        });
        let root_id = root.session_id().clone();

        let mut child = ChatSessionState::new();
        child.set_parent_session(root_id.clone());
        child.push_token_record(TokenRecord {
            model_used: None,
            timestamp: Timestamp::now(),
            tokens_sent: 300,
            tokens_received: 400,
            cost: Some(0.02),
            prompt_tokens: None,
            cached_tokens: None,
        });
        let child_id = child.session_id().clone();

        let mut grandchild = ChatSessionState::new();
        grandchild.set_parent_session(child_id.clone());
        grandchild.push_token_record(TokenRecord {
            model_used: None,
            timestamp: Timestamp::now(),
            tokens_sent: 500,
            tokens_received: 600,
            cost: Some(0.03),
            prompt_tokens: None,
            cached_tokens: None,
        });

        let mut map = HashMap::new();
        map.insert(root_id, root);
        map.insert(child_id, child);
        map.insert(grandchild.session_id().clone(), grandchild);
        map
    }

    #[rstest::rstest]
    fn aggregate_children_sums_own_and_descendants() {
        // Given a tree: root → child → grandchild.
        let map = build_tree();
        let root_id = map
            .iter()
            .find(|(_, s)| s.parent_session().is_none())
            .map(|(id, _)| id.clone())
            .expect("root");

        // When aggregating children of root.
        let stats = aggregate_children(&map, &root_id);

        // Then the result includes child (300+400) + grandchild (500+600).
        assert_eq!(stats.total_sent, 300 + 500); // 800
        assert_eq!(stats.total_received, 400 + 600); // 1000
        assert_eq!(stats.request_count, 2);
    }

    #[rstest::rstest]
    fn aggregate_children_returns_zero_for_leaf() {
        // Given a tree: root → child → grandchild.
        let map = build_tree();
        let grandchild_id = map
            .iter()
            .find(|(_, s)| {
                s.parent_session().is_some()
                    && !map
                        .values()
                        .any(|c| c.parent_session().as_ref() == Some(s.session_id()))
            })
            .map(|(id, _)| id.clone())
            .expect("grandchild");

        // When aggregating children of the grandchild (a leaf).
        let stats = aggregate_children(&map, &grandchild_id);

        // Then the result is zero (no descendants).
        assert_eq!(stats.total_sent, 0);
        assert_eq!(stats.total_received, 0);
        assert_eq!(stats.request_count, 0);
    }

    #[rstest::rstest]
    fn aggregate_children_cost_sums_all_descendant_costs() {
        // Given a tree: root → child → grandchild.
        let map = build_tree();
        let root_id = map
            .iter()
            .find(|(_, s)| s.parent_session().is_none())
            .map(|(id, _)| id.clone())
            .expect("root");

        // When aggregating costs of root's descendants.
        let cost = aggregate_children_cost(&map, &root_id);

        // Then child cost (0.02) + grandchild cost (0.03) = 0.05.
        let expected = 0.02 + 0.03;
        assert!(
            (cost - expected).abs() < f64::EPSILON,
            "expected {expected}, got {cost}"
        );
    }

    #[rstest::rstest]
    fn aggregate_children_cost_returns_zero_for_leaf() {
        // Given a tree: root → child → grandchild.
        let map = build_tree();
        let grandchild_id = map
            .iter()
            .find(|(_, s)| {
                s.parent_session().is_some()
                    && !map
                        .values()
                        .any(|c| c.parent_session().as_ref() == Some(s.session_id()))
            })
            .map(|(id, _)| id.clone())
            .expect("grandchild");

        // When aggregating costs of the grandchild (a leaf).
        let cost = aggregate_children_cost(&map, &grandchild_id);

        // Then the cost is 0.0.
        assert_eq!(cost, 0.0);
    }

    #[rstest::rstest]
    fn aggregate_session_stats_includes_own_and_children() {
        // Given a tree: root → child → grandchild.
        let map = build_tree();
        let root_id = map
            .iter()
            .find(|(_, s)| s.parent_session().is_none())
            .map(|(id, _)| id.clone())
            .expect("root");

        // When aggregating full stats for root.
        let stats = aggregate_session_stats(&map, &root_id);

        // Then own = root's record (100 sent, 200 received).
        assert_eq!(stats.own.total_sent, 100);
        assert_eq!(stats.own.total_received, 200);
        assert_eq!(stats.own.request_count, 1);

        // And children = child + grandchild (300+500 sent, 400+600 received).
        assert_eq!(stats.children.total_sent, 800);
        assert_eq!(stats.children.total_received, 1000);
        assert_eq!(stats.children.request_count, 2);

        // And costs are summed.
        assert!((stats.own_cost - 0.01).abs() < f64::EPSILON);
        assert!((stats.children_cost - 0.05).abs() < f64::EPSILON);
        assert!((stats.total_cost() - 0.06).abs() < f64::EPSILON);
    }

    #[rstest::rstest]
    fn aggregate_children_returns_zero_for_empty_map() {
        // Given an empty map.
        let map: HashMap<SessionId, ChatSessionState> = HashMap::new();
        let id = SessionId::new();

        // When aggregating.
        let stats = aggregate_children(&map, &id);

        // Then all zeros.
        assert_eq!(stats.total_sent, 0);
        assert_eq!(stats.total_received, 0);
        assert_eq!(stats.request_count, 0);
    }

    #[rstest::rstest]
    fn aggregate_children_cost_returns_zero_for_empty_map() {
        // Given an empty map.
        let map: HashMap<SessionId, ChatSessionState> = HashMap::new();
        let id = SessionId::new();

        // When aggregating costs.
        let cost = aggregate_children_cost(&map, &id);

        // Then zero.
        assert_eq!(cost, 0.0);
    }

    #[rstest::rstest]
    fn effective_sent_uses_provider_prompt_when_present_else_estimate() {
        // Given a ledger with one measured turn (provider prompt=120), one
        // cancelled turn (estimate only=50), and one turn where prompt matches.
        let records = vec![
            TokenRecord {
                model_used: None,
                timestamp: Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 10,
                cost: None,
                prompt_tokens: Some(120),
                cached_tokens: None,
            },
            TokenRecord {
                model_used: None,
                timestamp: Timestamp::now(),
                tokens_sent: 50,
                tokens_received: 0,
                cost: None,
                prompt_tokens: None,
                cached_tokens: None,
            },
        ];

        // When deriving stats.
        let stats = TokenStats::from_ledger(&records);

        // Then effective_sent = 120 (provider) + 50 (estimate fallback) = 170.
        assert_eq!(stats.effective_sent, 170);
    }

    #[rstest::rstest]
    fn cache_ratio_denominator_excludes_turns_without_usage() {
        // Given a ledger with a measured turn (prompt=1000, cached=400) and a
        // cancelled turn (no usage, estimate=50).
        let records = vec![
            TokenRecord {
                model_used: None,
                timestamp: Timestamp::now(),
                tokens_sent: 1000,
                tokens_received: 10,
                cost: None,
                prompt_tokens: Some(1000),
                cached_tokens: Some(400),
            },
            TokenRecord {
                model_used: None,
                timestamp: Timestamp::now(),
                tokens_sent: 50,
                tokens_received: 0,
                cost: None,
                prompt_tokens: None,
                cached_tokens: None,
            },
        ];

        // When deriving stats.
        let stats = TokenStats::from_ledger(&records);

        // Then measured_sent excludes the cancelled turn (1000, not 1050).
        assert_eq!(stats.measured_sent, 1000);
        // And cached_total reflects only measured turns (400).
        assert_eq!(stats.cached_total, 400);
    }
}
