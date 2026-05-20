//! Opens the lifecycle picker when the sessions section is focused.

use crate::common::app_state::AppState;

/// Handles `SidebarSessionNewWithLifecycle` — opens the lifecycle picker
/// when the sessions section is focused.
///
/// No-op if the sessions section is not focused.
pub fn handle_sidebar_session_new_with_lifecycle(
    state: &mut AppState,
) -> crate::protocol::IntentResult {
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::protocol::PickerKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return crate::protocol::IntentResult::empty();
    }
    crate::feat::picker::intent::handle_open_picker(state, PickerKind::SessionLifecycle)
}
