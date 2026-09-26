//! Global intent handlers - quit, toggle which-key, and interrupt.

use crate::common::app_state::AppState;
use crate::feat::chat_input::ChatInputBoxState;
use crate::protocol::{IntentResult, KernelIntent};
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
