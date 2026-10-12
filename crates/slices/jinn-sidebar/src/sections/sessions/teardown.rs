//! Session teardown handler.

use jinn_session_history_msg::PushChatEntry;

use crate::sections::sessions::close::validate_session_close;
use jinn_kernel::IntentResult;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::session_lifecycle::intent::build_run_session_teardown;

/// Handles `SidebarSessionTeardown` - re-runs teardown without closing the session.
///
/// Validates that the close can proceed (UI preconditions), resolves the
/// sidebar-selected session's ID, then delegates to
/// [`build_run_session_teardown`] which resolves + renders the teardown command
/// by session ID. If the session has no teardown command, this is a no-op. A
/// session whose recorded lifecycle file stopped resolving pushes a visible
/// error entry naming the file — a silent skip would leak whatever the
/// teardown was guarding.
///
/// # Panics
///
/// Does nothing if the sessions section has no cursor.
pub fn handle_session_teardown(
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) -> IntentResult {
    // Validate - same preconditions as session close.
    if validate_session_close(state).is_err() {
        return IntentResult::empty();
    }

    let Some(target_id) = state
        .frontend
        .with_sections(|s| s.sessions.selected_id.clone(), || None)
    else {
        return IntentResult::empty();
    };

    let Some(outcome) = build_run_session_teardown(state, &target_id, config) else {
        return IntentResult::empty();
    };
    match outcome {
        Ok(msg) => IntentResult::new_message(msg),
        Err(failure) => IntentResult::new_message(PushChatEntry {
            session_id: target_id,
            entry: jinn_core_types::ChatEntry::error(format!(
                "Teardown could not be resolved: {failure}"
            )),
            pin: None,
        }),
    }
}
