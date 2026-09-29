//! Re-running an attendant on the user's request.
//!
//! The `R` key in the sessions section calls into this from the frontend;
//! the logic lives here so the trigger path and the manual path share one
//! implementation.

use jinn_attendant_msg::AttendantActivation;
use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_core_types::SessionId;
use jinn_inference_msg::CancelStream;
use jinn_kernel::common::state::State;
use jinn_session_msg::PhaseKind;

use crate::activation;

/// The outcome of a successful rerun: cancel the in-flight turn (if it was
/// busy), then dispatch the seeded run.
pub type RerunOutcome = (Option<CancelStream>, Option<EnqueueUserMessage>);

/// Runs an attendant now, without any trigger condition.
///
/// Same sequence as a trigger fire: cancel the in-flight turn, reset context
/// per activation, seed, dispatch. Returns `None` when the run cannot start —
/// see [`rerun_blocked_reason`] for which reason applies.
pub fn rerun(state: &State, attendant_id: &SessionId) -> Option<RerunOutcome> {
    if rerun_blocked_reason(state, attendant_id).is_some() {
        return None;
    }
    let mut guard = state.write();
    rerun_in_state(&mut guard, attendant_id)
}

/// The keybind path: rerun against already-held mutable state.
///
/// The sidebar resolves the highlighted row and holds `&mut AppState`; the
/// trigger actor holds the shared [`State`]. Both run the same sequence, so
/// both delegate here with whatever access they have.
pub fn rerun_in_state(
    state: &mut jinn_kernel::AppState,
    attendant_id: &SessionId,
) -> Option<RerunOutcome> {
    if rerun_blocked_reason_in(state, attendant_id).is_some() {
        return None;
    }
    let session = state.session.get_mut(attendant_id)?;
    let cancel = (session.phase() != PhaseKind::Idle).then(|| CancelStream {
        session_id: attendant_id.clone(),
    });
    let dispatch = activation::prepare_run(session).map(|entry| {
        session.mark_turn_automated();
        EnqueueUserMessage {
            session_id: attendant_id.clone(),
            entry,
        }
    });
    Some((cancel, dispatch))
}

/// Why a rerun cannot start, for the status-bar hint.
///
/// `None` means the rerun is allowed.
#[must_use]
pub fn rerun_blocked_reason(state: &State, attendant_id: &SessionId) -> Option<&'static str> {
    let guard = state.read();
    rerun_blocked_reason_in(&guard, attendant_id)
}

/// The keybind path of [`rerun_blocked_reason`], over held state.
#[must_use]
pub fn rerun_blocked_reason_in(
    state: &jinn_kernel::AppState,
    attendant_id: &SessionId,
) -> Option<&'static str> {
    let session = state.session.get(attendant_id)?;
    if !session.is_attendant() {
        Some("not an attendant")
    } else if session.attendant_activation() == AttendantActivation::Seed {
        Some("attendant is still being composed (seed mode)")
    } else {
        None
    }
}
