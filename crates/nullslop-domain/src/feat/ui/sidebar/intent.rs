//! Sidebar-level intent handlers.
//!
//! Handles entering and leaving the sidebar scope. These are sidebar-panel
//! concerns, not section-specific — sections never handle ESC.

use crate::IntentResult;
use crate::common::app_state::AppState;

/// Handles `SidebarFocus` — enters sidebar scope.
///
/// Defaults focus to the Persona section (topmost section).
pub fn handle_sidebar_focus(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::{EnterFrom, SidebarSectionId};

    state.frontend.scope_stack.push(FocusScope::SidebarPersona);

    // If a section already has cursor state, restore it.
    let has_existing_cursor = state.frontend.persona_section.cursor.is_some()
        || state.frontend.pins.selected_id().is_some()
        || state.frontend.sessions_section.selected_index.is_some();

    if has_existing_cursor {
        // Restore to whichever section has a cursor.
        let section = if state.frontend.sessions_section.selected_index.is_some() {
            SidebarSectionId::Sessions
        } else if state.frontend.pins.selected_id().is_some() {
            SidebarSectionId::Pins
        } else {
            SidebarSectionId::Persona
        };
        state.frontend.scope_stack.set_sidebar_section(section);
    } else {
        // First entry — default to Persona at top.
        crate::feat::ui::sidebar::persona_section::receive_cursor(state, EnterFrom::Top);
    }

    IntentResult::empty()
}

/// Handles `SidebarLeave` — returns to Normal mode.
///
/// Always clears all overlay scopes, landing in Normal.
/// Does NOT set the cancel stream prompt — cancel confirmation
/// is handled exclusively by `NormalEscape`.
pub fn handle_sidebar_leave(state: &mut AppState) -> IntentResult {
    // Always return to Normal mode, regardless of previous scope.
    state.frontend.scope_stack.clear_overlays();
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::FocusScope;

    #[rstest::rstest]
    fn sidebar_focus_pushes_sidebar_scope() {
        // Given default app state.
        let mut state = AppState::default();

        // When handling sidebar focus.
        let result = handle_sidebar_focus(&mut state);

        // Then Sidebar is on the scope stack.
        assert!(state.frontend.scope_stack.is_sidebar());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_focus_defaults_to_persona() {
        // Given default app state.
        let mut state = AppState::default();

        // When handling sidebar focus.
        handle_sidebar_focus(&mut state);

        // Then persona section has the cursor.
        assert_eq!(state.frontend.persona_section.cursor, Some(0));
    }

    #[rstest::rstest]
    fn sidebar_focus_preserves_existing_cursor() {
        // Given a state with a pre-existing pins selection.
        use crate::protocol::{ChatEntry, PinPosition};

        let mut state = AppState::default();
        let entry = ChatEntry::user("test");
        let id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state.active_session_mut().pin_entry(&id, PinPosition::Top);
        state.frontend.pins.select_by_id(id);

        // When handling sidebar focus.
        handle_sidebar_focus(&mut state);

        // Then pins selection is preserved (not reset to persona).
        assert!(state.frontend.pins.selected_id().is_some());
        // And persona does NOT have cursor.
        assert!(state.frontend.persona_section.cursor.is_none());
    }

    #[rstest::rstest]
    fn sidebar_leave_returns_to_normal_scope() {
        // Given a state with Sidebar pushed onto the scope stack.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        // When handling sidebar leave.
        let result = handle_sidebar_leave(&mut state);

        // Then scope is back to Normal.
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_leave_from_input_always_returns_to_normal() {
        // Given a state that entered sidebar from Input mode.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Input);
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        // When handling sidebar leave.
        handle_sidebar_leave(&mut state);

        // Then scope is Normal (not Input).
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    }

    #[rstest::rstest]
    fn sidebar_leave_does_not_set_cancel_prompt_when_streaming() {
        // Given a state in Sidebar with an active stream.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);
        state.active_session_mut().begin_streaming();

        // When handling sidebar leave.
        let result = handle_sidebar_leave(&mut state);

        // Then no cancel prompt is set.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(result.commands.is_empty());
        // And scope is back to Normal.
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
    }
}
