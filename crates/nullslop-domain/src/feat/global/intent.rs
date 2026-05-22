//! Global intent handlers — quit, toggle which-key, and interrupt.

use crate::common::app_state::AppState;
use crate::feat::provider::protocol::command::CancelStream;
use crate::protocol::SessionId;
use crate::protocol::{Command, IntentResult};

use super::validator;

/// Handles the Quit intent.
///
/// Validates and sets `should_quit` on the frontend state.
pub fn handle_quit(state: &mut AppState) -> IntentResult {
    validator::validate_quit(state);
    state.frontend.should_quit = true;
    IntentResult::empty()
}

/// Handles the ToggleWhichkey intent.
///
/// Validates and sets the `toggle_whichkey` TUI signal.
pub fn handle_toggle_whichkey(state: &mut AppState) -> IntentResult {
    validator::validate_toggle_whichkey(state);
    state.frontend.tui_signals.toggle_whichkey = true;
    IntentResult::empty()
}

/// Handles the Interrupt intent.
///
/// When `target` is `None`, clears the input buffer.
/// When `target` is `Some(id)`, cancels the targeted session's stream
/// (for headless/scripted use).
pub fn handle_interrupt(state: &mut AppState, target: Option<&SessionId>) -> IntentResult {
    if let Some(id) = target {
        state.session_mut(id).cancel_streaming();
        return IntentResult::with_commands(vec![Command::CancelStream(CancelStream {
            session_id: id.clone(),
        })]);
    }

    // None path: just clear the input buffer.
    state.active_chat_input_mut().reset();
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::session::chat_session::SessionPhase;

    fn handle_quit(state: &mut AppState) -> IntentResult {
        super::handle_quit(state)
    }

    fn handle_toggle_whichkey(state: &mut AppState) -> IntentResult {
        super::handle_toggle_whichkey(state)
    }

    fn handle_interrupt(state: &mut AppState) -> IntentResult {
        super::handle_interrupt(state, None)
    }

    #[rstest::rstest]
    fn quit_sets_should_quit() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling Quit.
        let result = handle_quit(&mut state);

        // Then should_quit is true.
        assert!(state.frontend.should_quit);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn toggle_whichkey_sets_tui_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling ToggleWhichkey.
        let result = handle_toggle_whichkey(&mut state);

        // Then the toggle_whichkey signal is set.
        assert!(state.frontend.tui_signals.toggle_whichkey);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_clears_buffer_when_non_empty() {
        // Given a state with text in the buffer.
        let mut state = AppState::default();
        state.active_chat_input_mut().insert_grapheme_at_cursor('h');

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then the buffer is cleared.
        assert!(state.active_chat_input().is_empty());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_clears_empty_buffer_is_noop() {
        // Given a state with empty buffer.
        let mut state = AppState::default();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then no commands and buffer is still empty.
        assert!(state.active_chat_input().is_empty());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_does_not_cancel_stream() {
        // Given a state with empty buffer and active stream.
        let mut state = AppState::default();
        state.active_session_mut().begin_streaming();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then no CancelStream command is emitted.
        assert!(result.commands.is_empty());
        // And the session is still streaming.
        assert!(matches!(
            state.active_session().phase(),
            SessionPhase::Streaming
        ));
    }

    #[rstest::rstest]
    fn interrupt_with_specific_session_cancels_stream() {
        // Given two sessions, the second one streaming.
        use crate::protocol::SessionId;

        let mut state = AppState::default();
        let second_id = SessionId::new();
        let mut second_session = AppState::default();
        second_session.active_session_mut().begin_streaming();
        let mut second_session: crate::feat::session::chat_session::ChatSessionState =
            second_session
                .session
                .sessions_mut()
                .drain()
                .map(|(_, v)| v)
                .next()
                .unwrap();
        second_session.set_session_id(second_id.clone());
        state.session.insert(second_session);

        // When handling Interrupt targeting the second session.
        let result = super::handle_interrupt(&mut state, Some(&second_id));

        // Then the targeted session's stream is cancelled.
        assert!(matches!(
            state.session.get_unchecked(&second_id).phase(),
            SessionPhase::Idle
        ));
        // And a CancelStream command is returned for that session.
        assert_eq!(result.commands.len(), 1);
        assert!(
            matches!(&result.commands[0], Command::CancelStream (payload) if payload.session_id == second_id)
        );
    }
}
