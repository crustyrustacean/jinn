//! Reading the work-time cell, for the store actor's stamping points.
//!
//! The intervals live in the work-time slice's cell, and the monitor is their
//! only writer. The store actor reads them — never writes them — at the
//! moments a copy has to be made: when a session is persisted, and when it is
//! frozen into a tree node. Both copies are then immutable records of what
//! the monitor had recorded at that moment.
//!
//! The per-session read itself is the kernel's one cell-read helper
//! ([`jinn_kernel::common::session_persistence::working_intervals`]), shared
//! with the session-turn actor so every durable stamp reads the cell the same
//! way. This module keeps the aggregate read the tree needs.

use std::collections::HashMap;

use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_kernel::Services;
use jinn_work_time_msg::WorkingTimeState;
use jinn_work_time_msg::work_time_slot;

/// Every session's working intervals, keyed by session.
///
/// For the tree aggregate, which needs to union across a whole tree and
/// cannot reach the cell once it is reading `AppState` under a lock.
#[must_use]
pub fn all_working_intervals(
    services: &Services,
) -> std::collections::HashMap<SessionId, Vec<WorkingInterval>> {
    services
        .slices
        .reader::<WorkingTimeState>(&work_time_slot())
        .map_or_else(HashMap::new, |cell| cell.read().snapshot())
}
