//! Global intent handlers — quit, toggle which-key, and interrupt.

use nullslop_component::AppState;
use nullslop_protocol::provider::CancelStream;
use nullslop_protocol::{Command, IntentResult};

use crate::validator;

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
/// If the input buffer has text, clears it. If the buffer is empty and a
/// stream is active, cancels the stream and drains queued messages back
/// to the input buffer.
pub fn handle_interrupt(state: &mut AppState) -> IntentResult {
    if validator::validate_interrupt(state).is_err() {
        return IntentResult::empty();
    }

    state.active_chat_input_mut().deactivate_autocomplete();

    if state.active_chat_input().is_empty() {
        let session_id = state.session.active_session.clone();
        cancel_stream_and_drain(state);
        IntentResult::with_commands(vec![Command::CancelStream {
            payload: CancelStream { session_id },
        }])
    } else {
        state.active_chat_input_mut().reset();
        IntentResult::empty()
    }
}

/// Cancels streaming on the active session and drains any queued messages
/// back to the input buffer.
pub fn cancel_stream_and_drain(state: &mut AppState) {
    let session = state.active_session_mut();
    session.cancel_streaming();
    let drained: Vec<String> = session.drain_queue().into_iter().collect();
    let drained_text = drained.join("\n");
    if !drained_text.is_empty() {
        session.chat_input_mut().replace_all(drained_text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle_quit(state: &mut AppState) -> IntentResult {
        super::handle_quit(state)
    }

    fn handle_toggle_whichkey(state: &mut AppState) -> IntentResult {
        super::handle_toggle_whichkey(state)
    }

    fn handle_interrupt(state: &mut AppState) -> IntentResult {
        super::handle_interrupt(state)
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
    fn interrupt_resets_non_empty_buffer() {
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
    fn interrupt_cancels_stream_when_buffer_empty() {
        // Given a state with empty buffer and active stream.
        let mut state = AppState::default();
        state.active_session_mut().begin_streaming();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then a CancelStream command is returned.
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(&result.commands[0], Command::CancelStream { .. }));
        // And the session is idle (streaming was cancelled).
        assert!(state.active_session().is_idle());
    }

    #[rstest::rstest]
    fn interrupt_noop_when_idle_and_empty() {
        // Given a state with empty buffer and idle session.
        let mut state = AppState::default();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_drains_queued_messages_to_input_buffer() {
        // Given a streaming session with queued messages and empty input buffer.
        let mut state = AppState::default();
        state.active_session_mut().begin_streaming();
        state.active_session_mut().enqueue_message("queued1".into());
        state.active_session_mut().enqueue_message("queued2".into());

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then the queued messages are drained to the input buffer.
        assert_eq!(state.active_chat_input().text(), "queued1\nqueued2");
        // And the session is idle.
        assert!(state.active_session().is_idle());
        // And a CancelStream command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::CancelStream { .. }))
        );
    }
}
