//! The work-time slice's shared cell vocabulary: per-session working-time
//! intervals.

pub mod working_time_state;

pub use working_time_state::{RestoreWorkingTime, WorkingTimeState, work_time_slot};
