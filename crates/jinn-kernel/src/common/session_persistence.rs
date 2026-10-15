//! The one stamped-save flow every durable session write routes through.
//!
//! Two actors save sessions: the session-turn actor (every trigger on the
//! turn path — a prepared turn, a completed stream, a tool call, a pin
//! change, a task-list update, a steering drain) and the session-store actor
//! (rename persist, load reactivation, fork source). Both run the same
//! sequence — touch under a brief write lock, capture under a read lock,
//! stamp the snapshot with the work-time cell's current record, save — and
//! before this module each kept its own copy. The turn actor's copy did not
//! stamp, so every turn-path write stored empty intervals and silently wiped
//! the session's recorded wall time on each turn, while the store actor's
//! copy stamped. Same session blob, two behaviors.
//!
//! So the pairing lives here rather than in either actor, the way
//! `phase_events` does for the two phase writers: both actors' save
//! functions delegate to [`save_session_stamped`], and a future trigger site
//! that reaches either actor's save cannot store an un-stamped snapshot
//! without bypassing the flow deliberately. The interval read itself is
//! [`working_intervals`] — the single cell read behind every stamp.
//!
//! The monitor is the work-time cell's single writer; this module only ever
//! reads the cell. An absent cell is a working system, not a broken one: a
//! harness that never registered the catalog simply saves no intervals.

use jinn_core_types::SessionId;
use jinn_core_types::WorkingInterval;
use jinn_session_state::ChatSessionState;

use crate::common::services::Services;
use crate::common::state::State;

/// Saves a coherent snapshot of the session, stamped with its current wall
/// time.
///
/// `override_intervals` stands in for the cell read when supplied — the one
/// caller is the load path's reactivation write. The restore publish it
/// pairs with reaches the monitor asynchronously, so at that moment the cell
/// may still be empty; stamping from it would rewrite the metadata blob with
/// no intervals and erase exactly what the load just restored. The override
/// is the snapshot's own record — the same list the restore publish carries —
/// so the stored blob keeps what it already had.
///
/// The write lock is held only for the cheap timestamp mutation. The
/// follow-up read lock captures durable state before store I/O runs.
/// Failures are logged as warnings — persistence failure must not break the
/// user experience.
pub async fn save_session_stamped(
    services: &Services,
    state: &State,
    session_id: &SessionId,
    override_intervals: Option<Vec<WorkingInterval>>,
) {
    let store = &services.session_store;
    let state = state.clone();
    let requested_id = session_id.clone();
    let logged_id = session_id.clone();

    // Read the working intervals out of the work-time cell before the
    // blocking capture, so a running turn's open interval is written as
    // open and closes on load rather than being frozen at save time. The
    // load path's override stands in for this read — see the fn's doc.
    let working = match override_intervals {
        Some(intervals) => intervals,
        None => working_intervals(services, session_id),
    };
    let snapshot = tokio::task::spawn_blocking(move || {
        {
            state.with_session(|view| {
                if let Some(session) = view.session.map().get_mut(&requested_id) {
                    session.touch();
                }
            });
        }
        let state = state.read();
        state
            .session
            .get(&requested_id)
            .filter(|session| session.is_persistable())
            .map(ChatSessionState::capture_snapshot)
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(?error, "spawn_blocking panicked during session save");
        None
    });

    let Some(mut snapshot) = snapshot else { return };

    // Stamped here rather than read off the session core: the intervals
    // live in the work-time slice's cell, and the core deliberately does
    // not carry them, so there is exactly one copy and one writer.
    snapshot.metadata.working_intervals = working;

    if let Err(error) = store.save(&snapshot).await {
        tracing::warn!(
            session_id = ?logged_id,
            ?error,
            "failed to persist session"
        );
    }
}

/// This session's recorded working intervals, read from the work-time cell,
/// or an empty list when the cell is absent.
#[must_use]
pub fn working_intervals(services: &Services, session_id: &SessionId) -> Vec<WorkingInterval> {
    services
        .slices
        .reader::<jinn_work_time_msg::WorkingTimeState>(&jinn_work_time_msg::work_time_slot())
        .map_or_else(Vec::new, |cell| cell.read().intervals(session_id).to_vec())
}
