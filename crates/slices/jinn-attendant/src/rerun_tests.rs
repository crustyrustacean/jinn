//! Tests for the manual rerun action.

#![allow(clippy::expect_used, reason = "test code")]

use jinn_attendant_msg::AttendantActivation;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;
use jinn_session_msg::PhaseKind;
use jinn_session_state::ChatSessionState;

use crate::rerun::{rerun, rerun_blocked_reason};

fn state_with_attendant(activation: AttendantActivation) -> (State, jinn_core_types::SessionId) {
    let state = State::new(AppState::default());
    let id = {
        let mut guard = state.write();
        let mut attendant = ChatSessionState::new_attendant(&ChatSessionState::new(), true);
        attendant.set_attendant_activation(activation);
        let id = attendant.session_id().clone();
        guard.session.insert(attendant);
        id
    };
    (state, id)
}

#[rstest::rstest]
#[test]
fn rerun_on_a_reset_attendant_dispatches_the_seeded_run() {
    // Given a reset attendant with a prior report and a template.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.append_attendant_report("prior finding".to_owned());
        session.set_seed_template("verify: <prior report>".to_owned());
    }

    // When the attendant is re-run.
    let (cancel, dispatch) = rerun(&state, &id).expect("rerun allowed");

    // Then no cancel is needed (the session was idle) and the dispatch
    // carries the report through the template.
    assert!(cancel.is_none());
    let dispatch = dispatch.expect("reset mode dispatches");
    let jinn_core_types::chat_entry::ChatEntryKind::User { display, .. } = &dispatch.entry.kind
    else {
        panic!("seed must be a user entry");
    };
    assert_eq!(display, "verify: prior finding");
}

#[rstest::rstest]
#[test]
fn rerun_on_a_busy_attendant_cancels_its_own_turn_only() {
    // Given a reset attendant whose turn is mid-flight.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.begin_streaming();
    }

    // When the attendant is re-run.
    let (cancel, dispatch) = rerun(&state, &id).expect("rerun allowed");

    // Then the in-flight turn is cancelled and a new run dispatches.
    assert_eq!(cancel.expect("busy session cancels").session_id, id);
    assert!(dispatch.is_some());
    // And the session phase was observed, not mutated — the cancel is a
    // bus command the session actor applies.
    let guard = state.read();
    assert_eq!(guard.session.get(&id).expect("attendant").phase(), PhaseKind::Streaming);
}

#[rstest::rstest]
#[test]
fn rerun_on_a_seed_attendant_is_refused() {
    // Given an attendant still in seed mode.
    let (state, id) = state_with_attendant(AttendantActivation::Seed);

    // When the attendant is re-run.
    let outcome = rerun(&state, &id);

    // Then nothing dispatches, and the reason names the mode.
    assert!(outcome.is_none());
    assert_eq!(
        rerun_blocked_reason(&state, &id),
        Some("attendant is still being composed (seed mode)")
    );
}

#[rstest::rstest]
#[test]
fn rerun_on_a_non_attendant_session_is_refused() {
    // Given a plain user session.
    let state = State::new(AppState::default());
    let id = state.read().session.active_session_id().clone();

    // When it is re-run as an attendant.
    let outcome = rerun(&state, &id);

    // Then nothing dispatches, and the reason names the kind.
    assert!(outcome.is_none());
    assert_eq!(rerun_blocked_reason(&state, &id), Some("not an attendant"));
}
