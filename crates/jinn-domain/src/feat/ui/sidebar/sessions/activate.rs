//! Activates the session under the cursor.

use crate::common::app_state::AppState;

use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::IntentResult;

/// Activates the session under the cursor.
///
/// Called when the user presses Enter in the sessions section.
/// Uses `swap_base` to replace the entire scope stack, effectively
/// closing the sidebar and switching to the target view.
/// - For session entries: swaps to Normal (chat view). No re-scan commands
///   are emitted: each session's discovered skills/prompts/context-files
///   are ephemeral and persist across activation changes, and were
///   hydrated when the session was created/loaded.
/// - For plugin entries: if the plugin has a managed session, activates it. Otherwise no-op.
pub fn handle_session_activate(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return IntentResult::empty();
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };

    match entry.kind {
        SessionEntryKind::Session => {
            state.session.set_active(entry.id.clone());
            state.frontend.scope_stack.swap_base(FocusScope::Normal);
            IntentResult::empty()
        }
        SessionEntryKind::Plugin { .. } => {
            // Resolve the instance via its parent (origin) session and the
            // instance id carried in the entry id. The activate arm must
            // navigate to THIS instance's managed session, not the first
            // attachment with a matching name.
            let managed_id = entry
                .parent_id
                .as_ref()
                .and_then(|pid| state.session.get(pid))
                .and_then(|session| session.plugin_managed_session_id(&entry.id.to_string()));
            if let Some(managed_id) = managed_id {
                state.session.set_active(managed_id);
                state.frontend.scope_stack.swap_base(FocusScope::Normal);
            }
            IntentResult::empty()
        }
    }
}

/// Activates the session under the cursor and enters Insert mode.
///
/// Called when the user presses `i` in the sessions section. Like
/// [`handle_session_activate`] but lands in Input mode instead of Normal,
/// so the user can immediately start typing. The scope stack ends up
/// `[Normal, Input]` — Normal as the base so that ESC (`clear_overlays`)
/// correctly returns to Normal, with Input on top as the active mode.
/// - For session entries: activates the session, swaps to Normal as the
///   base, then pushes Input.
/// - For plugin entries: if the plugin has a managed session, activates it. Otherwise no-op.
pub fn handle_session_activate_insert(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return IntentResult::empty();
    }
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };

    match entry.kind {
        SessionEntryKind::Session => {
            state.session.set_active(entry.id.clone());
            state.frontend.scope_stack.swap_base(FocusScope::Normal);
            state.frontend.scope_stack.push(FocusScope::Input);
            IntentResult::empty()
        }
        SessionEntryKind::Plugin { .. } => {
            let managed_id = entry
                .parent_id
                .as_ref()
                .and_then(|pid| state.session.get(pid))
                .and_then(|session| session.plugin_managed_session_id(&entry.id.to_string()));
            if let Some(managed_id) = managed_id {
                state.session.set_active(managed_id);
                state.frontend.scope_stack.swap_base(FocusScope::Normal);
                state.frontend.scope_stack.push(FocusScope::Input);
            }
            IntentResult::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::focus::FocusScope;
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    use crate::protocol::SessionId;

    /// Two sessions exist in state; cursor points at the second-inserted session.
    fn state_with_two_sessions_cursor_on_second() -> (AppState, SessionId) {
        let mut state = AppState::default();
        let _first = state.session.active_session_id().clone();
        let second_session = crate::feat::session::chat_session::ChatSessionState::default();
        let second = second_session.session_id().clone();
        state.session.insert(second_session);
        // Cursor points at the second session in sorted order.
        let sessions = sorted_open_sessions(&state);
        let target_idx = sessions
            .iter()
            .position(|e| e.id == second)
            .expect("second session present");
        state.frontend.sessions_section.selected_index = Some(target_idx);
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        (state, second)
    }

    /// A session with two attached judge plugin instances, each holding its own
    /// managed (child) session. Cursor points at the SECOND plugin entry so we
    /// can verify activate resolves by instance id (not first-match-by-name).
    fn state_with_two_judge_instances_cursor_on_second_plugin() -> (AppState, SessionId) {
        use crate::feat::session::chat_session::ChatSessionState;
        use jinn_core_types::AttachedPlugin;

        let mut state = AppState::default();
        let origin = state.session.active_session_id().clone();

        // Two child sessions, one per judge instance.
        let child_a = {
            let s = ChatSessionState::default();
            let id = s.session_id().clone();
            state.session.insert(s);
            id
        };
        let child_b = {
            let s = ChatSessionState::default();
            let id = s.session_id().clone();
            state.session.insert(s);
            id
        };

        // Attach two judge instances to the origin, each with its own managed session.
        {
            let guard = state.session.get_mut(&origin).expect("origin");
            let mut a = AttachedPlugin::new("judge");
            a.managed_session_id = Some(child_a.clone());
            let mut b = AttachedPlugin::new("judge");
            b.managed_session_id = Some(child_b.clone());
            guard.attach_plugin(a);
            guard.attach_plugin(b);
        }
        // Cursor points at the second plugin entry in the built tree.
        let sessions = sorted_open_sessions(&state);
        let plugin_idx = sessions
            .iter()
            .filter(|e| matches!(e.kind, SessionEntryKind::Plugin { .. }))
            .nth(1)
            .and_then(|e| sessions.iter().position(|x| x.id == e.id))
            .expect("at least two plugin entries");
        state.frontend.sessions_section.selected_index = Some(plugin_idx);
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        (state, child_b)
    }

    #[rstest::rstest]
    fn activate_second_judge_instance_resolves_its_own_managed_session() {
        // Given a session with two judge instances, cursor on the second.
        let (mut state, expected_child) = state_with_two_judge_instances_cursor_on_second_plugin();

        // When activating.
        let _result = handle_session_activate(&mut state);

        // Then the active session is the SECOND instance's managed session,
        // not the first (proves instance-targeted resolution).
        assert_eq!(state.session.active_session_id(), &expected_child);
    }

    #[rstest::rstest]
    fn activate_session_switches_active_session_and_emits_no_commands() {
        // Given a sessions sidebar with cursor on a non-active session.
        let (mut state, expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then the active session is now the one under the cursor.
        assert_eq!(state.session.active_session_id(), &expected_id);
        // And no commands are emitted: each session's discovered
        // skills/prompts/context-files are ephemeral and were hydrated when the
        // session was created/loaded, so activation needs no re-scan.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn activate_session_with_no_cursor_emits_nothing() {
        // Given sessions sidebar but no selected index.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        // selected_index stays None.

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn activate_outside_sessions_section_emits_nothing() {
        // Given Normal scope (not sessions sidebar).
        let mut state = AppState::default();

        // When activating.
        let result = handle_session_activate(&mut state);

        // Then no commands emitted.
        assert!(result.message_names.is_empty());
    }
    #[rstest::rstest]
    fn activate_insert_switches_active_session() {
        // Given a sessions sidebar with cursor on a non-active session.
        let (mut state, expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the active session is now the one under the cursor.
        assert_eq!(state.session.active_session_id(), &expected_id);
    }

    #[rstest::rstest]
    fn activate_insert_pushes_input_with_normal_base() {
        // Given a sessions sidebar with cursor on a session.
        let (mut state, _expected_id) = state_with_two_sessions_cursor_on_second();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the top of the stack is Input (insert mode).
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Input);
        // And the base is Normal so ESC can return there via clear_overlays.
        assert_eq!(
            state.frontend.scope_stack.parent(),
            Some(&FocusScope::Normal)
        );
    }

    #[rstest::rstest]
    fn activate_insert_outside_sessions_section_is_noop() {
        // Given Normal scope (not sessions sidebar).
        let mut state = AppState::default();
        let initial_scope = state.frontend.scope_stack.current().clone();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the scope is unchanged.
        assert_eq!(state.frontend.scope_stack.current(), &initial_scope);
    }

    #[rstest::rstest]
    fn activate_insert_with_no_selected_index_is_noop() {
        // Given sessions sidebar but no selected index.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        let initial_scope = state.frontend.scope_stack.current().clone();

        // When activating into insert mode.
        handle_session_activate_insert(&mut state);

        // Then the scope is unchanged.
        assert_eq!(state.frontend.scope_stack.current(), &initial_scope);
    }
}
