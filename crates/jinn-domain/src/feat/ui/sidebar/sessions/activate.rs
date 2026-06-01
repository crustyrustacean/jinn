//! Activates the session or workflow under the cursor.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;

/// Activates the session or workflow under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// - For session entries: switches `active_session` and pushes Input scope.
/// - For workflow entries: sets the active workflow and pushes Workflow scope.
pub fn handle_session_activate(state: &mut AppState) {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return;
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return;
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return;
    };

    match entry.kind {
        SessionEntryKind::Session => {
            state.session.set_active(entry.id.clone());
            // Switch to insert mode so the user can start typing immediately.
            state.frontend.scope_stack.push(FocusScope::Input);
        }
        SessionEntryKind::Workflow => {
            let Some(wf_id) = &entry.workflow_id else {
                return;
            };
            if state.workflow.get(wf_id).is_some() {
                state.workflow.set_active(wf_id);
                state.frontend.scope_stack.push(FocusScope::Workflow);
            }
        }
    }
}
