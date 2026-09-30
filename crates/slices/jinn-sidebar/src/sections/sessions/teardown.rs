//! Session teardown handler.

use crate::sections::sessions::close::validate_session_close;
use jinn_kernel::IntentResult;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::session_lifecycle::intent::build_run_session_teardown;

/// Handles `SidebarSessionTeardown` - re-runs teardown without closing the session.
///
/// Validates that the close can proceed (UI preconditions), resolves the
/// sidebar-selected session's ID, then delegates to
/// [`build_run_session_teardown`] which resolves + renders the teardown command
/// by session ID. If the session has no teardown command, this is a no-op.
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

    let Some(msg) = build_run_session_teardown(state, &target_id, config) else {
        return IntentResult::empty();
    };
    IntentResult::new_message(msg)
}
