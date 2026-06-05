//! Activates the session or workflow under the cursor.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;

/// Activates the session or workflow under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Uses `swap_base` to replace the entire scope stack, effectively
/// closing the sidebar and switching to the target view.
/// - For session entries: swaps to Normal (chat view).
/// - For workflow entries: swaps to Workflow (graph view).
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
            state.frontend.scope_stack.swap_base(FocusScope::Normal);
        }
        SessionEntryKind::Plugin { .. } => {
            // Workflow entries are informational only; activating them is a no-op.
        }
    }
}
