//! Tests for the manual rerun action.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

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
    let (cancel, dispatch, _reset) = rerun(&state, &id).expect("rerun allowed");

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
fn rerun_on_a_busy_attendant_cancels_its_own_turn() {
    // Given a reset attendant whose turn is mid-flight.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.begin_streaming();
    }

    // When the attendant is re-run.
    let (cancel, dispatch, _reset) = rerun(&state, &id).expect("rerun allowed");

    // Then the in-flight turn is cancelled and a new run dispatches.
    assert_eq!(cancel.expect("busy session cancels").session_id, id);
    assert!(dispatch.is_some());
}

#[rstest::rstest]
#[test]
fn reset_run_excludes_non_pinned_entries_before_dispatching() {
    // Given a reset attendant with one pinned and two unpinned entries.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.push_entry(jinn_kernel::protocol::ChatEntry::user(
            "pinned instructions",
        ));
        let pinned_id = session.history()[0].id.clone();
        session.pin_entry(&pinned_id, jinn_core_types::PinPosition::Relative);
        session.push_entry(jinn_kernel::protocol::ChatEntry::assistant("an answer"));
        session.push_entry(jinn_kernel::protocol::ChatEntry::user("a follow-up"));
        session.set_seed_template("verify: <prior report>".to_owned());
        session.append_attendant_report("prior finding".to_owned());
    }

    // When the attendant is re-run.
    let (_, dispatch, _reset) = rerun(&state, &id).expect("rerun allowed");

    // Then the run still dispatches.
    assert!(dispatch.is_some(), "a reset run must still dispatch");
    // And every non-pinned entry is force-excluded from context, so the
    // model sees the pins alone.
    let guard = state.read();
    let session = guard.session.get(&id).expect("attendant");
    let history = session.history();
    assert_ne!(
        history[0].context_override(),
        jinn_core_types::ContextOverride::ForcedExclude,
        "the pinned entry must survive the reset"
    );
    assert_eq!(
        history[1].context_override(),
        jinn_core_types::ContextOverride::ForcedExclude
    );
    assert_eq!(
        history[2].context_override(),
        jinn_core_types::ContextOverride::ForcedExclude
    );
}

#[rstest::rstest]
#[case(AttendantActivation::Reset)]
#[case(AttendantActivation::Preserve)]
fn an_attendants_run_is_persistable_in_every_dispatchable_mode(#[case] mode: AttendantActivation) {
    // Given an attendant in the given mode.
    let (state, id) = state_with_attendant(mode);

    // When it is re-run.
    let (_, dispatch, _) = rerun(&state, &id).expect("rerun allowed");

    // Then the session is persistable, so the turn the run starts reaches
    // disk. An attendant that forgets its run on restart cannot be re-run
    // again, which is the whole feature.
    //
    // Persistability comes from the constructor rather than from the run, so
    // this is the property a mode switch has to preserve: editing the
    // activation in the properties popup must not drop the session out of
    // storage.
    let guard = state.read();
    assert!(
        guard.session.get(&id).expect("attendant").is_persistable(),
        "an attendant must stay persistable in {mode:?}"
    );
    // And the run was dispatched with the seeded prompt.
    assert!(dispatch.is_some(), "{mode:?} must dispatch a seeded run");
}

#[rstest::rstest]
fn reset_exclusions_survive_a_restart() {
    // Given a reset attendant that has been re-run once, so its context
    // was excluded in memory only.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.push_entry(jinn_kernel::protocol::ChatEntry::user("an answer"));
    }
    let (_, _, reset) = rerun(&state, &id).expect("rerun allowed");
    assert!(!reset.is_empty(), "the first reset has work to persist");

    // When the session is written to the store and read back into a fresh
    // shell, the way the load path rebuilds one.
    let guard = state.read();
    let session = guard.session.get(&id).expect("attendant");
    let written = session.capture_snapshot();
    let mut restored = jinn_session_state::ChatSessionState::new();
    restored.restore_history(written.entries);

    // Then the exclusion is still in force after the restart.
    let entry = restored.history()[0].clone();
    assert_eq!(
        entry.context_override(),
        jinn_core_types::ContextOverride::ForcedExclude
    );
}

#[rstest::rstest]
#[test]
fn rerun_on_a_busy_attendant_drops_its_own_phase_so_the_seed_dispatches() {
    // Given a reset attendant whose turn is mid-flight.
    let (state, id) = state_with_attendant(AttendantActivation::Reset);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.begin_streaming();
    }

    // When the attendant is re-run.
    let _ = rerun(&state, &id).expect("rerun allowed");

    // Then its own phase is back to Idle.
    //
    // This is the invariant the enqueue handler depends on: a session that
    // is still `Sending`/`Streaming` *queues* an incoming user message
    // instead of dispatching it, so leaving the phase hot would make `R`
    // silently queue the seed rather than run it. The frontend owns its own
    // session's phase (the same contract `Esc` relies on), and the session
    // actor owns the descendants' — hence the cascade publishes messages
    // rather than writing their phases.
    let guard = state.read();
    assert_eq!(
        guard.session.get(&id).expect("attendant").phase(),
        PhaseKind::Idle
    );
}

#[rstest::rstest]
#[test]
fn rerun_on_a_preserve_attendant_seeds_through_the_template() {
    // Given a preserve-mode attendant with a prior report and a template.
    //
    // `R` is the user saying "ask again", so it goes through the template
    // and inserts the seeded message in every mode. Preserve mode governs
    // what an unattended trigger fire does — carry the context as-is — not
    // what a manual re-run does.
    let (state, id) = state_with_attendant(AttendantActivation::Preserve);
    {
        let mut guard = state.write();
        let session = guard.session.get_mut(&id).expect("attendant");
        session.append_attendant_report("prior finding".to_owned());
        session.set_seed_template("verify: <prior report>".to_owned());
    }

    // When the attendant is re-run.
    let (_, dispatch, _) = rerun(&state, &id).expect("rerun allowed");

    // Then the seeded run dispatches with the report substituted.
    let dispatch = dispatch.expect("R must seed in every non-seed mode");
    let jinn_core_types::chat_entry::ChatEntryKind::User { display, .. } = &dispatch.entry.kind
    else {
        panic!("seed must be a user entry");
    };
    assert_eq!(display, "verify: prior finding");
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
