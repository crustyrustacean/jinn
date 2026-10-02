//! Lightweight projections derived from complete live or captured session state.

use jinn_core_types::WorkingInterval;
use jinn_session_store_msg::FrozenTreeNode;
use jinn_token_count_msg::TokenStats;

use crate::{ChatSessionState, SessionSnapshot, compute_turn_count};

/// Creates the archived tree projection for one live session.
///
/// `working_intervals` is passed in rather than read from the session: the
/// intervals live in the work-time slice's cell, which the session does not
/// own and must not reach into. The caller is the session-store actor, which
/// snapshots the cell at freeze time.
#[must_use]
pub fn snapshot_frozen_node(
    session: &ChatSessionState,
    working_intervals: Vec<WorkingInterval>,
) -> FrozenTreeNode {
    let token_stats = TokenStats::from_ledger(session.token_ledger());
    FrozenTreeNode {
        session_id: session.session_id().clone(),
        parent_session_id: session.parent_session().clone(),
        total_sent: token_stats.total_sent,
        total_received: token_stats.total_received,
        total_cost: TokenStats::total_cost(session.token_ledger()),
        total_turns: compute_turn_count(session.history(), session.fork_ordinal()),
        effective_sent: token_stats.effective_sent,
        measured_sent: token_stats.measured_sent,
        cached_total: token_stats.cached_total,
        working_intervals,
    }
}

/// Creates the archived tree projection for one complete snapshot.
#[must_use]
pub fn snapshot_frozen_node_from_snapshot(snapshot: &SessionSnapshot) -> FrozenTreeNode {
    let token_stats = TokenStats::from_ledger(&snapshot.token_ledger);
    FrozenTreeNode {
        session_id: snapshot.metadata.session_id.clone(),
        parent_session_id: snapshot.metadata.parent_session.clone(),
        total_sent: token_stats.total_sent,
        total_received: token_stats.total_received,
        total_cost: TokenStats::total_cost(&snapshot.token_ledger),
        total_turns: compute_turn_count(&snapshot.entries, snapshot.metadata.fork_ordinal),
        effective_sent: token_stats.effective_sent,
        measured_sent: token_stats.measured_sent,
        cached_total: token_stats.cached_total,
        // Carried out of the snapshot, so a tree member that was never loaded
        // into memory still contributes the time it worked.
        working_intervals: snapshot.metadata.working_intervals.clone(),
    }
}
