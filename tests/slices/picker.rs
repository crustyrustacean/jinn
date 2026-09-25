//! End-to-end crossing tests for the picker family.
//!
//! The twelve picker specs now live in `jinn-picker-specs` and the generic
//! dispatch layer (intents, host lens, geometry) stays in the kernel. These
//! tests cross that boundary the way production does: the real registry is
//! built by `jinn_picker_specs::build_picker_registry`, the kernel's
//! open/confirm dispatch drives it, and each test asserts one observable
//! result.
//!
//! The session picker is the subject because its whole chain is real and
//! side-effect-free: opening enters the picker scope, and confirming begins
//! the load for the selected session.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use jinn_core_types::SessionId;
use jinn_domain::AppState;
use jinn_domain::feat::ui::picker_states::PickerExt;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::SessionState;
use jinn_session_store_msg::SessionTreeEntry;
use jinn_slices::FocusScope;
use jinn_slices::picker_kind::PickerKind;

/// A state with two sessions inserted, so the picker has something to switch
/// to. Returns the state and the *other* session's id.
fn state_with_two_sessions() -> (AppState, SessionId) {
    let mut state = AppState::default_with_scope_focus();
    let origin = ChatSessionState::new();
    state.session.insert(origin);
    state
        .session
        .set_active(state.session.active_session_id().clone());

    let other = ChatSessionState::new();
    let other_id = other.session_id().clone();
    state.session.insert(other);

    (state, other_id)
}

/// A picker row for `session_id`, shaped like the rows the session actor
/// builds when it loads the store.
fn session_entry(state: &AppState, session_id: SessionId, title: &str) -> SessionTreeEntry {
    SessionTreeEntry::new(
        session_id,
        title.to_owned(),
        jiff::Timestamp::now(),
        state.frontend.theme.clone(),
        SessionState::Loaded,
        None,
        None,
    )
}

#[rstest::rstest]
fn open_session_picker_enters_picker_scope() {
    // Given a state outside any picker, and the real picker registry.
    let registry = jinn_picker_specs::build_picker_registry();
    let mut state = AppState::default_with_scope_focus();
    let scopes_before = state.frontend.scope_len();

    // When opening the session picker through the kernel's dispatch.
    jinn_domain::feat::picker::intent::handle_open_picker(
        &mut state,
        PickerKind::Session,
        &registry,
    );

    // Then the picker scope is on top of the scope stack.
    assert_eq!(state.frontend.scope_len(), scopes_before + 1);
    assert!(state.frontend.is_picker());
}

#[rstest::rstest]
fn confirm_session_picker_begins_loading_the_selected_session() {
    // Given a state with two sessions and the session picker open, with the
    // second session highlighted and wrapped through the real registry.
    let registry = jinn_picker_specs::build_picker_registry();
    let (mut state, other_id) = state_with_two_sessions();
    let active_id = state.session.active_session_id().clone();
    let entries = vec![
        session_entry(&state, active_id, "first"),
        session_entry(&state, other_id.clone(), "second"),
    ];
    let wrapped = registry
        .make_items(jinn_picker::SESSION_ID, entries)
        .expect("session spec is registered");
    state.frontend.session_picker_mut().set_items(wrapped);
    state.frontend.session_picker_mut().move_down(1);
    state.frontend.scope_push(FocusScope::Picker {
        kind: PickerKind::Session,
    });

    // When confirming through the kernel's dispatch.
    let (result, _) =
        jinn_domain::feat::picker::intent::handle_picker_confirm(&mut state, &registry);

    // Then the load began for the selected (second) session.
    let guard = state
        .session
        .session_load_guard()
        .expect("confirming the picker begins the load");
    assert_eq!(guard.session_id, other_id);
    assert!(
        result
            .message_names
            .iter()
            .any(|name| name.contains("SessionLoadRequested")),
        "confirming emits the session load request"
    );
}
