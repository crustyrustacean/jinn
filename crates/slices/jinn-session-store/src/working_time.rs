//! Reading the work-time cell, for the store actor's two stamping points.
//!
//! The intervals live in the work-time slice's cell, and the monitor is their
//! only writer. The store actor reads them — never writes them — at the two
//! moments a copy has to be made: when a session is persisted, and when it is
//! frozen into a tree node. Both copies are then immutable records of what
//! the monitor had recorded at that moment.
//!
//! An absent cell is a working system, not a broken one: a harness that never
//! registered the catalog has no cell, and the session is then simply reported
//! as having worked no recorded time rather than failing a save.

use std::collections::HashMap;

use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_kernel::Services;
use jinn_work_time_msg::WorkingTimeState;
use jinn_work_time_msg::work_time_slot;

/// This session's recorded working intervals, or an empty list when the cell
/// is absent.
#[must_use]
pub fn working_intervals(services: &Services, session_id: &SessionId) -> Vec<WorkingInterval> {
    services
        .slices
        .reader::<WorkingTimeState>(&work_time_slot())
        .map_or_else(Vec::new, |cell| cell.read().intervals(session_id).to_vec())
}

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
