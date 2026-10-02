//! Per-session working-time intervals, held in the work-time slice's cell.
//!
//! A session is working exactly when its phase is not idle, and the working-time
//! monitor is the only writer here. It never reads a phase: it folds the
//! `WorkStateChanged` events the phase writers publish, so a turn whose
//! boundaries it never observed costs it nothing rather than a guess.
//!
//! Intervals are kept coalesced as they are written, so a turn that hands off
//! through an intermediate phase reads as one stretch rather than several
//! slivers.

use std::collections::HashMap;

use jiff::Timestamp;
use jinn_core_types::{CoalescingGap, SessionId, WorkingInterval, coalesce, total};

use jinn_slices::SlotKey;

/// The coalescing threshold used on write, in one place so no caller can
/// apply a second threshold.
const WRITE_GAP: CoalescingGap = CoalescingGap::two_seconds();

/// Every session's recorded working intervals, keyed by session.
///
/// Readers take a snapshot of the map they need rather than holding a guard:
/// the status bar reads it every frame, and cloning a handful of small vectors
/// is cheaper than contending a lock the monitor writes on every phase change.
#[derive(Debug, Clone, Default)]
pub struct WorkingTimeState {
    intervals: HashMap<SessionId, Vec<WorkingInterval>>,
}

impl WorkingTimeState {
    /// Whether this session currently has an open interval.
    #[must_use]
    pub fn is_working(&self, session_id: &SessionId) -> bool {
        self.intervals
            .get(session_id)
            .is_some_and(|list| list.iter().any(WorkingInterval::is_open))
    }

    /// This session's intervals, or an empty slice when it has never worked.
    #[must_use]
    pub fn intervals(&self, session_id: &SessionId) -> &[WorkingInterval] {
        self.intervals.get(session_id).map_or(&[], Vec::as_slice)
    }

    /// A snapshot of every session's intervals.
    ///
    /// For a consumer that must union across a whole tree: the tree's members
    /// are decided elsewhere, and cloning lets that decision and the union
    /// happen without holding this state's guard.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<SessionId, Vec<WorkingInterval>> {
        self.intervals.clone()
    }

    /// This session's working time, measured to `at` while it is still working.
    #[must_use]
    pub fn working_time(&self, session_id: &SessionId, at: Timestamp) -> jiff::SignedDuration {
        total(self.intervals(session_id), at)
    }

    /// Opens an interval at `at`, unless one is already open.
    ///
    /// A no-op on an already-working session, so a repeated `working: true`
    /// event cannot open a second interval and double-bill the overlap.
    pub fn begin_working(&mut self, session_id: SessionId, at: Timestamp) {
        let list = self.intervals.entry(session_id).or_default();
        if list.iter().any(WorkingInterval::is_open) {
            return;
        }
        list.push(WorkingInterval::open(at));
        coalesce(list, WRITE_GAP);
    }

    /// Closes this session's open interval at `at`.
    ///
    /// Returns `true` when an interval was closed. A no-op when the session
    /// has none open, so a forced `Idle → Idle` event — which is not a
    /// boundary — cannot close a second time.
    pub fn end_working(&mut self, session_id: &SessionId, at: Timestamp) -> bool {
        let Some(list) = self.intervals.get_mut(session_id) else {
            return false;
        };
        let closed = list
            .iter_mut()
            .rev()
            .find_map(|interval| interval.close(at).then_some(()))
            .is_some();
        if closed {
            coalesce(list, WRITE_GAP);
        }
        closed
    }

    /// Replaces this session's intervals with `intervals`, coalescing them.
    ///
    /// The load path's write: a session restored from disk brings its recorded
    /// time with it, and a pre-existing snapshot carrying none starts empty
    /// rather than being treated as having never worked.
    pub fn restore(&mut self, session_id: SessionId, mut intervals: Vec<WorkingInterval>) {
        if intervals.is_empty() {
            return;
        }
        coalesce(&mut intervals, WRITE_GAP);
        self.intervals.insert(session_id, intervals);
    }
}

/// The slot key the work-time slice's cell lives under.
#[must_use]
pub fn work_time_slot() -> SlotKey {
    SlotKey::builtin("work-time", "intervals")
}

/// A session's recorded working intervals, as loaded from storage.
///
/// Published by the session-store actor when a session is restored, so the
/// monitor — not the store — puts them back in the cell. The store reads
/// working time to stamp a durable copy; letting it also write one would give
/// working intervals a second writer, and the two would disagree about the
/// open interval a killed session left behind.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, trouper::schema::Command)]
#[schema(description = "A loaded session's recorded working intervals.")]
pub struct RestoreWorkingTime {
    /// The session whose intervals were loaded.
    pub session_id: SessionId,
    /// The intervals storage held for it.
    pub intervals: Vec<WorkingInterval>,
}

impl jinn_slices::BusMessage for RestoreWorkingTime {}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]
    use super::*;
    use jiff::SignedDuration;

    const START: i64 = 1_000_000_000;

    fn at(offset_secs: i64) -> Timestamp {
        Timestamp::from_second(START + offset_secs).expect("valid offset")
    }

    #[rstest::rstest]
    fn beginning_work_opens_an_interval() {
        // Given a state with no record for a session.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();

        // When the session starts working.
        state.begin_working(id.clone(), at(0));

        // Then it holds one open interval from that moment.
        assert!(state.is_working(&id));
        assert_eq!(state.intervals(&id).len(), 1);
        assert!(state.intervals(&id)[0].is_open());
    }

    #[rstest::rstest]
    fn beginning_work_twice_does_not_open_a_second_interval() {
        // Given a session already working.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.begin_working(id.clone(), at(0));

        // When a second start arrives.
        state.begin_working(id.clone(), at(3));

        // Then no second interval was opened, so the overlap is not double-billed.
        assert_eq!(state.intervals(&id).len(), 1, "{:?}", state.intervals(&id));
    }

    #[rstest::rstest]
    fn ending_work_closes_the_open_interval() {
        // Given a session working since 0.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.begin_working(id.clone(), at(0));

        // When it stops at 10.
        let closed = state.end_working(&id, at(10));

        // Then the interval closed and the session is no longer working.
        assert!(closed);
        assert!(!state.is_working(&id));
        assert_eq!(
            state.working_time(&id, at(100)),
            SignedDuration::from_secs(10)
        );
    }

    #[rstest::rstest]
    fn ending_work_on_an_idle_session_reports_no_close() {
        // Given a state that never saw this session work.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();

        // When a stop arrives anyway.
        let closed = state.end_working(&id, at(10));

        // Then nothing was closed, so a forced idle event is a no-op.
        assert!(!closed);
    }

    #[rstest::rstest]
    fn ending_work_twice_closes_only_once() {
        // Given a session that stopped at 10.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.begin_working(id.clone(), at(0));
        assert!(state.end_working(&id, at(10)));

        // When a second stop arrives at 20.
        let closed = state.end_working(&id, at(20));

        // Then the first end stands and the total is not extended.
        assert!(!closed);
        assert_eq!(
            state.working_time(&id, at(100)),
            SignedDuration::from_secs(10)
        );
    }

    #[rstest::rstest]
    fn a_working_session_keeps_accruing_until_it_stops() {
        // Given a session working since 0.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.begin_working(id.clone(), at(0));

        // When its working time is read 10 seconds later.
        let working = state.working_time(&id, at(10));

        // Then the time is live, not frozen at the start.
        assert_eq!(working, SignedDuration::from_secs(10));
    }

    #[rstest::rstest]
    fn two_turns_close_by_within_the_gap_are_one_interval() {
        // Given a session that worked 0..3, idled 2s, then worked 5..8.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.begin_working(id.clone(), at(0));
        state.end_working(&id, at(3));
        state.begin_working(id.clone(), at(5));
        state.end_working(&id, at(8));

        // Then the list is a single coalesced interval.
        assert_eq!(state.intervals(&id).len(), 1, "{:?}", state.intervals(&id));
    }

    #[rstest::rstest]
    fn restored_intervals_survive_into_the_total() {
        // Given a state restored with a session's recorded interval.
        let mut state = WorkingTimeState::default();
        let id = SessionId::new();
        state.restore(id.clone(), vec![WorkingInterval::closed(at(0), at(12))]);

        // Then the total reads the restored span.
        assert_eq!(
            state.working_time(&id, at(100)),
            SignedDuration::from_secs(12)
        );
    }

    #[rstest::rstest]
    fn restoring_nothing_leaves_the_session_unknown() {
        // Given a state with no record for a session.
        let state = WorkingTimeState::default();
        let id = SessionId::new();

        // When its working time is read.
        let working = state.working_time(&id, at(10));

        // Then it is zero rather than an error.
        assert_eq!(working, SignedDuration::ZERO);
    }
}
