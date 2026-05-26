//! Tree-wide aggregate statistics for session trees.
//!
//! When sessions form a tree (parent/child via `parent_session`), this module
//! provides functions to find the tree root and compute aggregate stats across
//! the entire tree — not just the active session's descendants.

use std::collections::{HashMap, HashSet};

use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::token_stats::TokenStats;
use crate::feat::ui::status_bar::turn_counter;
use crate::protocol::SessionId;

/// Aggregate statistics for an entire session tree.
///
/// Sums tokens, cost, and turns across ALL sessions in the tree (root + all
/// descendants), regardless of which session is currently active.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeAggregateStats {
    /// Total tokens sent across all sessions in the tree.
    pub total_sent: u64,
    /// Total tokens received across all sessions in the tree.
    pub total_received: u64,
    /// Total cost across all sessions in the tree.
    pub total_cost: f64,
    /// Total turns across all sessions in the tree.
    pub total_turns: u32,
    /// Number of sessions in the tree.
    pub session_count: usize,
}

/// Find the root of the session tree containing `session_id`.
///
/// Walks up via `parent_session` links until reaching a session with no parent
/// (or whose parent is not in the sessions map). Guards against cycles with a
/// visited set.
///
/// Returns `session_id` itself if it has no parent (single session or root).
pub fn find_tree_root<S>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    session_id: &SessionId,
) -> SessionId
where
    S: std::hash::BuildHasher,
{
    let mut visited = HashSet::new();
    let mut current = session_id.clone();

    loop {
        // Cycle protection: if we've visited this node, it's the root.
        if !visited.insert(current.clone()) {
            return current;
        }

        // Look up the current session.
        let Some(session) = sessions.get(&current) else {
            // Session not found in map — treat as root.
            return current;
        };

        // Check for parent.
        let Some(parent_id) = session.parent_session().as_ref() else {
            // No parent — this is the root.
            return current;
        };

        // Check that the parent exists in the map.
        if !sessions.contains_key(parent_id) {
            // Orphan — parent was removed/archived. This node is the root.
            return current;
        }

        current = parent_id.clone();
    }
}

/// Compute aggregate statistics for the entire session tree containing `session_id`.
///
/// 1. Finds the tree root via [`find_tree_root`].
/// 2. Collects ALL sessions in the tree (BFS from root).
/// 3. Sums token stats, cost, and turns.
pub fn aggregate_tree_stats<S>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    session_id: &SessionId,
) -> TreeAggregateStats
where
    S: std::hash::BuildHasher,
{
    let root = find_tree_root(sessions, session_id);

    // BFS to collect all sessions in the tree.
    let mut tree_sessions = Vec::new();
    let mut queue = vec![root];
    let mut visited = HashSet::new();

    while let Some(id) = queue.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(session) = sessions.get(&id) {
            tree_sessions.push(session);
            // Find children: sessions whose parent_session points to this id.
            for (child_id, child) in sessions {
                if child.parent_session().as_ref() == Some(&id)
                    && !visited.contains(child_id)
                {
                    queue.push(child_id.clone());
                }
            }
        }
    }

    // Aggregate stats.
    let mut stats = TreeAggregateStats {
        session_count: tree_sessions.len(),
        ..Default::default()
    };

    for session in &tree_sessions {
        let token_stats = TokenStats::from_ledger(session.token_ledger());
        stats.total_sent += token_stats.total_sent;
        stats.total_received += token_stats.total_received;
        stats.total_cost += TokenStats::total_cost(session.token_ledger());
        stats.total_turns += turn_counter::compute_turn_count(session.history());
    }

    stats
}
