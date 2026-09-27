//! Global intent handlers - quit, toggle which-key, and interrupt.

use crate::common::app_state::AppState;
use crate::protocol::{IntentResult, KernelIntent};
use jinn_chat_input_msg::ChatInputBoxState;
use jinn_core_types::SessionId;
use jinn_inference_msg::CancelStream;

use super::validator;

/// Handles the Quit intent.
///
/// Validates and latches the quit request on the frontend state.
pub fn handle_quit(state: &mut AppState) -> IntentResult {
    validator::validate_quit(state);
    state.frontend.set_quit(true);
    IntentResult::empty()
}

/// Handles the ToggleWhichkey intent.
///
/// Validates and sets the `toggle_whichkey` TUI signal.
pub fn handle_toggle_whichkey(state: &mut AppState) -> IntentResult {
    validator::validate_toggle_whichkey(state);
    state
        .frontend
        .update_scope(|s| s.signals.toggle_whichkey = true);
    IntentResult::empty()
}

/// Handles the `ToggleAuditPopup` intent.
///
/// Flips the global `audit_popup_visible` flag on `FrontendState`. Always
/// succeeds (no validator). The popup is rendered by the TUI layer when the
/// flag is `true`.
pub fn handle_toggle_audit_popup(state: &mut AppState) -> IntentResult {
    state.frontend.audit_popup_visible = !state.frontend.audit_popup_visible;
    IntentResult::empty()
}

/// Handles the Interrupt intent.
///
/// When `target` is `None`, clears the input buffer.
/// When `target` is `Some(id)`, cancels the targeted session's stream
/// (for headless/scripted use).
pub fn handle_interrupt(state: &mut AppState, target: Option<&SessionId>) -> IntentResult {
    if let Some(id) = target {
        state
            .session_mut(id)
            .cancel_streaming(jiff::Timestamp::now());
        return IntentResult::new_message(CancelStream {
            session_id: id.clone(),
        });
    }

    // None path: just clear the input buffer.
    state.update_active_input(ChatInputBoxState::reset);
    IntentResult::empty()
}

/// Handles the `CtrlClear` intent (universal `<c-c>` clear-or-leave).
///
/// Clears chat input, or clears/closes a picker filter. Input-capturing
/// slice popups bind their own clear-or-leave actions.
pub fn handle_ctrl_clear(state: &mut AppState) -> (IntentResult, Option<KernelIntent>) {
    use jinn_slices::FocusScope;

    match state.frontend.scope() {
        FocusScope::Input => {
            state.update_active_input(ChatInputBoxState::reset);
            (IntentResult::empty(), None)
        }
        // No kernel picker exists: every picker is slice-owned and binds
        // <c-c> in its own scope, so a `Picker` focus scope here means a
        // legacy scope name that no longer resolves. The slice-owned key wins.
        _ => (IntentResult::empty(), None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::items_after_statements,
        reason = "test code"
    )]

    use super::*;
    use jinn_session_msg::PhaseKind;
    use jinn_slices::FocusScope;

    fn handle_quit(state: &mut AppState) -> IntentResult {
        super::handle_quit(state)
    }

    fn handle_toggle_whichkey(state: &mut AppState) -> IntentResult {
        super::handle_toggle_whichkey(state)
    }

    fn handle_toggle_audit_popup(state: &mut AppState) -> IntentResult {
        super::handle_toggle_audit_popup(state)
    }

    fn handle_interrupt(state: &mut AppState) -> IntentResult {
        super::handle_interrupt(state, None)
    }

    #[rstest::rstest]
    fn quit_sets_should_quit() {
        // Given a default state.
        let mut state = AppState::default_with_scope_focus();

        // When handling Quit.
        let result = handle_quit(&mut state);

        // Then the quit latch is set.
        assert!(state.frontend.quit());
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn toggle_whichkey_sets_tui_signal() {
        // Given a default state.
        let mut state = AppState::default_with_scope_focus();

        // When handling ToggleWhichkey.
        let result = handle_toggle_whichkey(&mut state);

        // Then the toggle_whichkey signal is set.
        assert!(state.frontend.signals_snapshot().toggle_whichkey);
        assert!(result.message_names.is_empty());
    }
    #[rstest::rstest]
    fn toggle_audit_popup_off_to_on_sets_visibility_flag() {
        // Given a default state (popup hidden).
        let mut state = AppState::default_with_scope_focus();
        assert!(!state.frontend.audit_popup_visible);

        // When toggling once.
        let result = handle_toggle_audit_popup(&mut state);

        // Then the flag flips to true.
        assert!(state.frontend.audit_popup_visible);
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn toggle_audit_popup_on_to_off_clears_visibility_flag() {
        // Given a state with the popup toggled on.
        let mut state = AppState::default_with_scope_focus();
        handle_toggle_audit_popup(&mut state);
        assert!(state.frontend.audit_popup_visible);

        // When toggling a second time.
        let result = handle_toggle_audit_popup(&mut state);

        // Then the flag flips back to false.
        assert!(!state.frontend.audit_popup_visible);
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn audit_popup_remains_visible_when_input_mode_entered() {
        // Given a state with the audit popup toggled on, scoped to Normal mode.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_swap_base(jinn_slices::FocusScope::Normal);
        handle_toggle_audit_popup(&mut state);
        assert!(state.frontend.audit_popup_visible);

        // When the user enters Input mode (pushes Input focus scope).
        state.frontend.scope_push(jinn_slices::FocusScope::Input);

        // Then the popup flag remains on — it lives on FrontendState, not Mode.
        assert!(state.frontend.audit_popup_visible);
        // And the scope stack reflects Input mode (the `a` keybind is not
        // registered in Input mode, so the toggle cannot be flipped from here).
        assert_eq!(state.frontend.scope(), FocusScope::Input);
    }

    #[rstest::rstest]
    fn audit_popup_remains_visible_after_input_mode_exited() {
        // Given a state with the popup toggled on, scoped to Normal, then Input mode entered.
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_swap_base(jinn_slices::FocusScope::Normal);
        handle_toggle_audit_popup(&mut state);
        state.frontend.scope_push(jinn_slices::FocusScope::Input);
        assert!(state.frontend.audit_popup_visible);

        // When the user pops back to Normal.
        state.frontend.scope_pop();

        // Then the flag still persists.
        assert!(state.frontend.audit_popup_visible);
        assert_eq!(state.frontend.scope(), FocusScope::Normal);
    }

    #[rstest::rstest]
    fn interrupt_clears_buffer_when_non_empty() {
        // Given a state with text in the buffer.
        let mut state = AppState::default_with_scope_focus();
        state.update_active_input(|i| i.insert_grapheme_at_cursor('h'));

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then the buffer is cleared.
        assert!(
            state
                .active_session()
                .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
        );
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_clears_empty_buffer_is_noop() {
        // Given a state with empty buffer.
        let mut state = AppState::default_with_scope_focus();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then no commands and buffer is still empty.
        assert!(
            state
                .active_session()
                .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
        );
        assert!(result.message_names.is_empty());
    }

    #[rstest::rstest]
    fn interrupt_does_not_cancel_stream() {
        // Given a state with empty buffer and active stream.
        let mut state = AppState::default_with_scope_focus();
        state.active_session_mut().begin_streaming();

        // When handling Interrupt.
        let result = handle_interrupt(&mut state);

        // Then no CancelStream command is emitted.
        assert!(result.message_names.is_empty());
        // And the session is still streaming.
        assert!(matches!(
            state.active_session().phase(),
            PhaseKind::Streaming
        ));
    }

    #[rstest::rstest]
    fn interrupt_with_specific_session_cancels_stream() {
        // Given two sessions, the second one streaming.
        use jinn_core_types::SessionId;

        let mut state = AppState::default_with_scope_focus();
        let second_id = SessionId::new();
        let mut second_session = jinn_session_state::ChatSessionState::new();
        second_session.set_session_id(second_id.clone());
        second_session.begin_streaming();
        state.session.insert(second_session);

        // When handling Interrupt targeting the second session.
        let result = super::handle_interrupt(&mut state, Some(&second_id));

        // Then the targeted session's stream is cancelled.
        assert!(matches!(
            state.session.get_unchecked(&second_id).phase(),
            PhaseKind::Idle
        ));
        // And a CancelStream message is returned for that session.
        assert_eq!(result.messages.len(), 1);
    }

    // ============================================================
    // CtrlClear tests
    // ============================================================

    fn handle_ctrl_clear(state: &mut AppState) -> (IntentResult, Option<KernelIntent>) {
        super::handle_ctrl_clear(state)
    }

    #[rstest::rstest]
    fn ctrl_clear_input_nonempty_clears_buffer() {
        // Given a state in Input scope with text in the buffer.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Input);
        state.update_active_input(|i| i.insert_grapheme_at_cursor('h'));
        state.update_active_input(|i| i.insert_grapheme_at_cursor('i'));

        // When handling CtrlClear.
        let (result, maybe_intent) = handle_ctrl_clear(&mut state);

        // Then the buffer is cleared and no redispatch is requested.
        assert!(
            state
                .active_session()
                .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
        );
        assert!(result.message_names.is_empty());
        assert!(maybe_intent.is_none());
    }

    #[rstest::rstest]
    fn ctrl_clear_input_empty_is_noop() {
        // Given a state in Input scope with empty buffer.
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Input);

        // When handling CtrlClear.
        let (result, maybe_intent) = handle_ctrl_clear(&mut state);

        // Then no commands, no redispatch, scope unchanged.
        assert!(
            state
                .active_session()
                .with_input(jinn_chat_input_msg::ChatInputBoxState::is_empty, || true)
        );
        assert!(result.message_names.is_empty());
        assert!(maybe_intent.is_none());
        assert_eq!(state.frontend.scope(), FocusScope::Input);
    }

    #[rstest::rstest]
    fn ctrl_clear_in_a_legacy_picker_scope_is_a_no_op() {
        // Given a state pushed into a legacy `Picker` focus scope — a scope
        // name no kernel picker resolves any more.
        use crate::protocol::PickerKind;
        let mut state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::CompactionModel,
        });

        // When handling CtrlClear.
        let (result, maybe_intent) = handle_ctrl_clear(&mut state);

        // Then nothing happens: the kernel does not close it, and it does not
        // claim the filter either. Every real picker clears its own filter in
        // its own scope, so the kernel staying out of it is the contract.
        assert!(result.message_names.is_empty());
        assert!(maybe_intent.is_none());
        // And the scope is untouched, so the slice's own key stays in charge.
        assert!(state.frontend.is_picker());
    }
}
