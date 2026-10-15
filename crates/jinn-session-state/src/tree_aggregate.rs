//! Tree-wide aggregate statistics for session trees.
//!
//! When sessions form a tree (parent/child via `parent_session`), this module
//! provides functions to find the tree root and compute aggregate stats across
//! the entire tree - not just the active session's descendants.
//!
//! Archived sessions are represented as lightweight [`FrozenTreeNode`] snapshots
//! that preserve tree structure and stats without holding full session state.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_core_types::union;
use jinn_token_count_msg::{TokenStats, TreeAggregateStats};

use crate::{ChatSessionState, compute_turn_count};
pub use crate::{snapshot_frozen_node, snapshot_frozen_node_from_snapshot};
pub use jinn_session_store_msg::FrozenTreeNode;

/// Find the root of the session tree containing `session_id`.
///
/// Walks up via `parent_session` links until reaching a session with no parent
/// (or whose parent is not in any map). Checks both live sessions and frozen
/// nodes to resolve parent links across archived sessions. Guards against
/// cycles with a visited set.
///
/// Returns `session_id` itself if it has no parent (single session or root).
pub fn find_tree_root<S: ::std::hash::BuildHasher>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    frozen_nodes: &HashMap<SessionId, FrozenTreeNode, S>,
    session_id: &SessionId,
) -> SessionId {
    let mut visited = HashSet::new();
    let mut current = session_id.clone();

    loop {
        // Cycle protection: if we've visited this node, it's the root.
        if !visited.insert(current.clone()) {
            return current;
        }

        // Look up parent from live session or frozen node.
        let parent_id = if let Some(session) = sessions.get(&current) {
            session.parent_session().clone()
        } else if let Some(frozen) = frozen_nodes.get(&current) {
            frozen.parent_session_id.clone()
        } else {
            // Not found anywhere - treat as root.
            return current;
        };

        let Some(parent_id) = parent_id else {
            // No parent - this is the root.
            return current;
        };

        // Check that the parent exists in either live sessions or frozen nodes.
        if !sessions.contains_key(&parent_id) && !frozen_nodes.contains_key(&parent_id) {
            // Orphan - parent was removed. This node is the root.
            return current;
        }

        current = parent_id;
    }
}

/// Compute aggregate statistics for the entire session tree containing `session_id`.
///
/// 1. Finds the tree root via [`find_tree_root`].
/// 2. Collects ALL sessions in the tree (BFS from root), including frozen nodes.
/// 3. Sums token stats, cost, and turns.
///
/// `live_intervals` carries the working intervals of the *live* members, which
/// the aggregate cannot read from the sessions themselves: they live in the
/// work-time slice's cell, and duplicating them onto the session would give
/// working time a second home with a second writer. Frozen members carry their
/// own, stamped at freeze time. A member absent from `live_intervals`
/// contributes no working time.
pub fn aggregate_tree_stats<S: ::std::hash::BuildHasher>(
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    frozen_nodes: &HashMap<SessionId, FrozenTreeNode, S>,
    session_id: &SessionId,
    live_intervals: &HashMap<SessionId, Vec<WorkingInterval>>,
) -> TreeAggregateStats {
    let root = find_tree_root(sessions, frozen_nodes, session_id);
    let members = collect_tree_members(sessions, frozen_nodes, root, live_intervals);

    let mut stats = TreeAggregateStats::default();
    aggregate_live_sessions(&mut stats, &members.live);
    aggregate_frozen_nodes(&mut stats, &members.frozen);
    stats.session_count = members.live.len() + members.frozen.len();
    stats.total_working = union_member_intervals(&members.intervals);
    stats
}

/// Every session and frozen node reached from the root, with the working
/// intervals each member contributes.
struct TreeMembers<'a> {
    live: Vec<&'a ChatSessionState>,
    frozen: Vec<&'a FrozenTreeNode>,
    intervals: Vec<Vec<WorkingInterval>>,
}

/// BFS from the root collecting all sessions and frozen nodes in the tree.
fn collect_tree_members<'a, S: ::std::hash::BuildHasher>(
    sessions: &'a HashMap<SessionId, ChatSessionState, S>,
    frozen_nodes: &'a HashMap<SessionId, FrozenTreeNode, S>,
    root: SessionId,
    live_intervals: &HashMap<SessionId, Vec<WorkingInterval>>,
) -> TreeMembers<'a> {
    let mut members = TreeMembers {
        live: Vec::new(),
        frozen: Vec::new(),
        intervals: Vec::new(),
    };
    let mut queue = vec![root];
    let mut visited = HashSet::new();

    while let Some(id) = queue.pop() {
        if !visited.insert(id.clone()) {
            continue;
        }
        // A member absent from both maps cannot be enqueued by
        // `enqueue_children_of`, so the fallthrough below is unreachable;
        // matching with an else would be dead code.
        #[expect(
            clippy::else_if_without_else,
            reason = "no-op on fallthrough is intentional"
        )]
        if let Some(session) = sessions.get(&id) {
            members.live.push(session);
            if let Some(intervals) = live_intervals.get(&id) {
                members.intervals.push(intervals.clone());
            }
        } else if let Some(frozen) = frozen_nodes.get(&id) {
            members.frozen.push(frozen);
            members.intervals.push(frozen.working_intervals.clone());
        }
        enqueue_children_of(&id, sessions, frozen_nodes, &mut queue, &visited);
    }

    members
}

/// Schedules every child of `id` — live or frozen — that the BFS has not
/// yet visited. Both member kinds resolve `parent_session`, so the two
/// scans are one step, not one per arm.
fn enqueue_children_of<S: ::std::hash::BuildHasher>(
    id: &SessionId,
    sessions: &HashMap<SessionId, ChatSessionState, S>,
    frozen_nodes: &HashMap<SessionId, FrozenTreeNode, S>,
    queue: &mut Vec<SessionId>,
    visited: &HashSet<SessionId>,
) {
    for (child_id, child) in sessions {
        if child.parent_session().as_ref() == Some(id) && !visited.contains(child_id) {
            queue.push(child_id.clone());
        }
    }
    for (frozen_id, frozen) in frozen_nodes {
        if frozen.parent_session_id.as_ref() == Some(id) && !visited.contains(frozen_id) {
            queue.push(frozen_id.clone());
        }
    }
}

/// Sums token stats, cost, and turns across the tree's live sessions.
fn aggregate_live_sessions(stats: &mut TreeAggregateStats, live: &[&ChatSessionState]) {
    for session in live {
        let token_stats = TokenStats::from_ledger(session.token_ledger());
        stats.total_sent += token_stats.total_sent;
        stats.total_received += token_stats.total_received;
        stats.total_cost += TokenStats::total_cost(session.token_ledger());
        stats.total_turns += compute_turn_count(session.history(), session.fork_ordinal());
        stats.effective_sent += token_stats.effective_sent;
        stats.measured_sent += token_stats.measured_sent;
        stats.cached_total += token_stats.cached_total;
    }
}

/// Sums the frozen stats each archived member stamped at freeze time.
fn aggregate_frozen_nodes(stats: &mut TreeAggregateStats, frozen: &[&FrozenTreeNode]) {
    for node in frozen {
        stats.total_sent += node.total_sent;
        stats.total_received += node.total_received;
        stats.total_cost += node.total_cost;
        stats.total_turns += node.total_turns;
        stats.effective_sent += node.effective_sent;
        stats.measured_sent += node.measured_sent;
        stats.cached_total += node.cached_total;
    }
}

/// Working time is the UNION of every member's intervals, not their sum.
/// A parent blocked on two subagents is itself non-idle, so summing would
/// bill the same wall-clock second three times. Union is also what makes
/// archiving a member harmless: its frozen node carries the intervals it
/// contributed while live, so the total does not move.
fn union_member_intervals(member_intervals: &[Vec<WorkingInterval>]) -> jiff::SignedDuration {
    let at = Timestamp::now();
    let borrowed: Vec<&[WorkingInterval]> = member_intervals.iter().map(Vec::as_slice).collect();
    union(&borrowed, at)
}
