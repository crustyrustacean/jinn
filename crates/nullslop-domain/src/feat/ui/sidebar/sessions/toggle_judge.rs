//! Toggle a judge session's attached/detached state from the sidebar.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::IntentResult;

/// Handles `ToggleJudgeAttached` — toggles `is_attached` on the judge session
/// under the sidebar cursor.
///
/// If the selected session is not a judge session, this is a silent no-op.
/// The sidebar re-renders on the next frame, reflecting the visual change
/// (purple bg for attached, lavender fg for detached).
pub fn handle_toggle_judge_attached(state: &mut AppState) -> IntentResult {
    // Guard: must be in the Sessions sidebar section.
    if !matches!(
        state.frontend.scope_stack.sidebar_section(),
        Some(SidebarSectionId::Sessions)
    ) {
        return IntentResult::empty();
    }

    // Guard: a session must be selected.
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return IntentResult::empty();
    };

    // Get the selected entry from the sorted sessions snapshot.
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };

    // Guard: only judge sessions can be toggled.
    if !entry.is_judge {
        return IntentResult::empty();
    }

    let session_id = entry.id.clone();

    // Drop the immutable borrow before mutating.
    drop(sessions);

    // Toggle is_attached on the underlying ChatSessionState.
    let Some(session) = state.session.get_mut(&session_id) else {
        return IntentResult::empty();
    };

    let Some(judge_meta) = session.judge().as_ref() else {
        return IntentResult::empty();
    };
    let current = judge_meta.is_attached;
    // Release the immutable borrow on `session` before mutating.
    let _ = judge_meta;
    session.set_judge_attached(!current);

    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::judge::JudgeMeta;
    use crate::feat::session::chat_session::ChatSessionState;

    /// Helper: create an AppState with an origin session and a judge child session.
    /// Returns (state, judge_id).
    fn state_with_judge(is_attached: bool) -> (AppState, crate::protocol::SessionId) {
        let mut state = AppState::default();
        let origin_id = state.session.active_session_id().clone();

        let mut judge_session = ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached,
            judge_name: "test-judge".to_string(),
auto_reset: None,
});
        // Set the parent so sorted_open_sessions places the judge as a child
        // of the origin: [origin, judge].
        judge_session.set_parent_session(origin_id);
        state.session.insert(judge_session);

        (state, judge_id)
    }

    #[rstest::rstest]
    fn toggle_attached_to_detached() {
        let (mut state, judge_id) = state_with_judge(true);

        // Focus sidebar on sessions, select the judge entry.
        // sorted_open_sessions returns: [origin, judge_child] → index 1 is the judge.
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let result = handle_toggle_judge_attached(&mut state);

        let guard = state.session.get(&judge_id).expect("judge session exists");
        assert!(!guard.judge().as_ref().expect("has judge meta").is_attached);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn toggle_detached_to_attached() {
        let (mut state, judge_id) = state_with_judge(false);

        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let _result = handle_toggle_judge_attached(&mut state);

        let guard = state.session.get(&judge_id).expect("judge session exists");
        assert!(guard.judge().as_ref().expect("has judge meta").is_attached);
    }

    #[rstest::rstest]
    fn noop_on_non_judge() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(0);

        let result = handle_toggle_judge_attached(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_wrong_section() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        let result = handle_toggle_judge_attached(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_no_selection() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        assert!(state.frontend.sessions_section.selected_index.is_none());

        let result = handle_toggle_judge_attached(&mut state);

        assert!(result.commands.is_empty());
    }
}
