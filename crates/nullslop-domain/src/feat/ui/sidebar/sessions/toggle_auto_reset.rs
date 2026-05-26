//! Toggle a judge session's per-session auto-reset override from the sidebar.

use crate::common::app_state::AppState;
use crate::feat::judge::resolve_effective_auto_reset;
use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::IntentResult;

/// Handles `ToggleJudgeAutoReset` — toggles the per-session `auto_reset` override
/// on the judge session under the sidebar cursor.
///
/// The effective auto-reset is resolved (per-session override → judge file default).
/// The override is then flipped: `None` becomes `Some(!default)`, `Some(x)` becomes `Some(!x)`.
///
/// If the selected session is not a judge session, this is a silent no-op.
/// The sidebar re-renders on the next frame, reflecting the visual change
/// (↺ indicator for auto-reset active).
pub fn handle_toggle_auto_reset(state: &mut AppState) -> IntentResult {
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

    // Resolve current effective auto-reset, then flip it.
    let Some(session) = state.session.get(&session_id) else {
        return IntentResult::empty();
    };
    let Some(judge_meta) = session.judge().as_ref() else {
        return IntentResult::empty();
    };
    let current_effective =
        resolve_effective_auto_reset(judge_meta, &state.context.judges);
    // Release the immutable borrow on `session` before mutating.
    let _ = session;

    let new_override = Some(!current_effective);
    let Some(session) = state.session.get_mut(&session_id) else {
        return IntentResult::empty();
    };
    session.set_judge_auto_reset(new_override);

    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::judge::{Judge, JudgeMeta};
    use crate::feat::session::chat_session::ChatSessionState;

    /// Helper: create an AppState with an origin session, a judge child session,
    /// and a registered judge with `auto_reset = false` (file default).
    /// Returns (state, judge_id).
    fn state_with_judge(session_override: Option<bool>) -> (AppState, crate::protocol::SessionId) {
        let mut state = AppState::default();
        let origin_id = state.session.active_session_id().clone();

        // Register a judge in context with auto_reset = false.
        state.context.judges.push(Judge {
            name: "test-judge".to_string(),
            description: String::new(),
            body: String::new(),
            model: None,
            auto_reset: false,
            file_path: std::path::PathBuf::from("test-judge.md"),
        });

        let mut judge_session = ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_string(),
            auto_reset: session_override,
        });
        judge_session.set_parent_session(origin_id);
        state.session.insert(judge_session);

        (state, judge_id)
    }

    #[rstest::rstest]
    fn toggle_from_none_to_some_true() {
        // Judge file default is false, per-session is None → effective is false.
        // Toggle should set override to Some(true).
        let (mut state, judge_id) = state_with_judge(None);

        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let _result = handle_toggle_auto_reset(&mut state);

        let guard = state.session.get(&judge_id).expect("judge session exists");
        let meta = guard.judge().as_ref().expect("has judge meta");
        assert_eq!(meta.auto_reset, Some(true)); // Overridden to true
    }

    #[rstest::rstest]
    fn toggle_from_some_true_to_some_false() {
        let (mut state, judge_id) = state_with_judge(Some(true));

        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let _result = handle_toggle_auto_reset(&mut state);

        let guard = state.session.get(&judge_id).expect("judge session exists");
        let meta = guard.judge().as_ref().expect("has judge meta");
        assert_eq!(meta.auto_reset, Some(false)); // Flipped to false
    }

    #[rstest::rstest]
    fn noop_on_non_judge() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(0);

        let result = handle_toggle_auto_reset(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_wrong_section() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        let result = handle_toggle_auto_reset(&mut state);

        assert!(result.commands.is_empty());
    }
}
