//! Reset a judge session's history from the sidebar.
//!
//! Truncates the judge session's history to only its pinned entries,
//! preserving the same `SessionId`, `JudgeMeta`, model, CWD, and parent linkage.
//! Rejects with a system message to the origin session if the judge is actively working.

use crate::ChatEntry;
use crate::common::app_state::AppState;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::session::chat_session::SessionPhase;
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use crate::protocol::{Command, IntentResult};

/// System message shown when reset is rejected because the judge is busy.
const BUSY_REJECTION_MESSAGE: &str = "Cannot reset judge while it is working.";

/// Handles `ResetJudge` — truncates the judge session's history to pinned entries only.
///
/// Guards:
/// - Must be in the Sessions sidebar section.
/// - A session must be selected.
/// - The selected entry must be a judge session.
///
/// If the judge is actively working (non-Idle phase or `is_busy()`),
/// pushes a system message to the origin session and returns without resetting.
///
/// Otherwise, calls [`crate::feat::session::chat_session::ChatSessionState::reset_judge_history()`]
/// and emits a `PersistSession` command.
pub fn handle_reset_judge(state: &mut AppState) -> IntentResult {
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

    // Guard: only judge sessions can be reset.
    if !entry.is_judge {
        return IntentResult::empty();
    }

    let session_id = entry.id.clone();

    // Drop the immutable borrow before mutating.
    drop(sessions);

    // Get mutable access to the judge session.
    let Some(judge_session) = state.session.get_mut(&session_id) else {
        return IntentResult::empty();
    };

    // Extract origin session ID before checking busy state.
    let Some(judge_meta) = judge_session.judge().as_ref() else {
        return IntentResult::empty();
    };
    let origin_session_id = judge_meta.origin_session.clone();

    // Guard: reject if the judge is actively working.
    let phase = judge_session.phase();
    let is_busy = judge_session.is_busy();
    if phase != SessionPhase::Idle || is_busy {
        // Push a system message to the ORIGIN session (not the judge).
        return IntentResult::with_commands(vec![Command::PushChatEntry(PushChatEntry {
            session_id: origin_session_id,
            entry: ChatEntry::system(BUSY_REJECTION_MESSAGE),
        })]);
    }

    // Perform the reset.
    judge_session.reset_judge_history();

    // Persist the truncated state.
    IntentResult::with_commands(vec![Command::PersistSession(PersistSession { session_id })])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::judge::JudgeMeta;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::chat_session::{ChatSessionState, SessionPhase};
    use crate::protocol::PinPosition;

    /// Helper: create an AppState with an origin session and a judge child session.
    /// The judge has a pinned system entry and some non-pinned entries.
    /// Returns (state, judge_id, origin_id).
    fn state_with_idle_judge() -> (AppState, crate::protocol::SessionId, crate::protocol::SessionId) {
        let mut state = AppState::default();
        let origin_id = state.session.active_session_id().clone();

        let mut judge_session = ChatSessionState::new();
        let judge_id = judge_session.session_id().clone();
        judge_session.set_judge(JudgeMeta {
            origin_session: origin_id.clone(),
            is_attached: true,
            judge_name: "test-judge".to_string(),
        });
        judge_session.set_parent_session(origin_id.clone());

        // Push a pinned system entry (judge body).
        judge_session.push_entry(
            ChatEntry::system("You are a judge.").with_pin(PinPosition::Top),
        );
        // Push non-pinned entries.
        judge_session.push_entry(ChatEntry::user("evaluate this"));
        judge_session.push_entry(ChatEntry::system("verdict: pass"));

        state.session.insert(judge_session);

        (state, judge_id, origin_id)
    }

    #[rstest::rstest]
    fn resets_idle_judge_successfully() {
        let (mut state, judge_id, _origin_id) = state_with_idle_judge();

        // Focus sidebar on sessions, select the judge entry (index 1 = judge child).
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let result = handle_reset_judge(&mut state);

        // Judge history should have only the pinned system entry.
        let guard = state.session.get(&judge_id).expect("judge session exists");
        assert_eq!(guard.history().len(), 1);
        assert!(guard.history()[0].is_pinned());

        // A PersistSession command should be emitted.
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(&result.commands[0], Command::PersistSession(p) if p.session_id == judge_id));
    }

    #[rstest::rstest]
    fn rejects_busy_judge_with_system_message() {
        let (mut state, judge_id, origin_id) = state_with_idle_judge();

        // Set phase to Streaming (non-Idle).
        state.session.get_mut(&judge_id).expect("judge").core.ephemeral.phase = SessionPhase::Streaming;

        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let result = handle_reset_judge(&mut state);

        // Judge history should be unchanged (3 entries).
        let guard = state.session.get(&judge_id).expect("judge session exists");
        assert_eq!(guard.history().len(), 3);

        // A PushChatEntry command should be emitted to the ORIGIN session.
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(&result.commands[0], Command::PushChatEntry(p) if p.session_id == origin_id));
    }

    #[rstest::rstest]
    fn rejects_busy_counter_judge_with_system_message() {
        let (mut state, judge_id, origin_id) = state_with_idle_judge();

        // Phase is Idle but busy_counter is set.
        {
            let judge = state.session.get_mut(&judge_id).expect("judge");
            assert_eq!(judge.phase(), SessionPhase::Idle);
            judge.core.ephemeral.busy_counter.set_busy();
            assert!(judge.is_busy());
        }

        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(1);

        let result = handle_reset_judge(&mut state);

        // Judge history should be unchanged.
        let guard = state.session.get(&judge_id).expect("judge session exists");
        assert_eq!(guard.history().len(), 3);

        // Rejection message to origin.
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(&result.commands[0], Command::PushChatEntry(p) if p.session_id == origin_id));
    }

    #[rstest::rstest]
    fn noop_on_non_judge() {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state.frontend.sessions_section.selected_index = Some(0);

        let result = handle_reset_judge(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_wrong_section() {
        let (mut state, _judge_id, _origin_id) = state_with_idle_judge();
        state.frontend.scope_stack.push(FocusScope::SidebarPersona);

        let result = handle_reset_judge(&mut state);

        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn noop_no_selection() {
        let (mut state, _judge_id, _origin_id) = state_with_idle_judge();
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        assert!(state.frontend.sessions_section.selected_index.is_none());

        let result = handle_reset_judge(&mut state);

        assert!(result.commands.is_empty());
    }
}
