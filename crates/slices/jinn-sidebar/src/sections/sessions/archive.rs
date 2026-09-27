//! Archive session handler.

use crate::sections::sessions::close::validate_session_close;
use crate::sections::sessions::state::{mark_in_flight, sorted_open_sessions};
use jinn_kernel::common::app_state::AppState;

/// Handles `SidebarSessionArchive` - archives the selected session without teardown.
///
/// Validates that the close can proceed, then emits an `ArchiveSession` command.
/// The actor handles DB archival and memory removal.
///
/// # Panics
/// Panics if `sessions_section.selected_index` is `None`.
pub fn handle_session_archive(state: &mut AppState) -> jinn_kernel::protocol::IntentResult {
    use jinn_session_store_msg::ArchiveSession;

    // Validate - same preconditions as session close.
    if validate_session_close(state).is_err() {
        return jinn_kernel::protocol::IntentResult::empty();
    }

    let index = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)
        .unwrap();
    let sessions = sorted_open_sessions(state);
    let Some(target) = sessions.get(index) else {
        return jinn_kernel::protocol::IntentResult::empty();
    };
    let target_id = target.id.clone();

    // Mark in flight - the row stays tinted until the archive concludes.
    mark_in_flight(state, std::slice::from_ref(&target_id));

    // Emit ArchiveSession - the actor handles archival without teardown.
    jinn_kernel::protocol::IntentResult::new_message(ArchiveSession {
        session_id: target_id,
    })
}
