//! IntentHandler arms for the terminal tab (takeover UI).
//!
//! Three intents:
//! - [`Intent::TerminalTakeControl`] — `i` in TerminalView: flips the shared
//!   control flag to *user* (synchronously, so an in-flight tool call's drain
//!   sees the takeover on its next iteration — mailbox ordering cannot
//!   deliver that) and pushes [`FocusScope::TerminalControl`].
//! - [`Intent::TerminalSendKey`] — catch-all routing in TerminalControl: the
//!   encoded bytes go straight to the pty via [`IntentHandlerCommand::SendTermKey`].
//! - [`Intent::TerminalHandback`] — handback key in TerminalControl: flips
//!   control back to the agent, pops to TerminalView, and steers the captured
//!   screen to the model (busy → steering buffer drained at next dispatch;
//!   idle → dispatched immediately by the queue actor).
//!
//! Per the style guide, the IntentHandler is the exempt frontend mutator: it
//! may write `frontend.terminal.control` (and the shared flag) even though
//! the coordinator actor also writes the mirror — that's the documented
//! optimistic-write + authoritative-write pattern.

use crate::common::app_state::{AppState, FocusScope};
use crate::feat::interactive_term::interactive_term_actor::TermControl;
use crate::feat::interactive_term::protocol::command::ControlHolder;
use crate::feat::interactive_term::terminal_tab_state::TermControlHolder;
use crate::feat::tools_actor::interactive_term_send::USER_HAS_CONTROL_NOTICE;

/// The shared control flag installed by actor wiring. Set once at startup;
/// before that, takeover intents no-op (the tab renders an empty screen).
pub static TERM_CONTROL: std::sync::OnceLock<TermControl> = std::sync::OnceLock::new();

/// Returns the shared control flag, if installed.
fn control() -> Option<&'static TermControl> {
    TERM_CONTROL.get()
}

/// Handles [`Intent::TerminalTakeControl`].
pub fn handle_take_control(state: &mut AppState) -> crate::protocol::intent::IntentResult {
    if let Some(flag) = control() {
        flag.set(ControlHolder::User);
    }
    state.frontend.terminal.set_control(TermControlHolder::User);
    state.frontend.scope_stack.push(FocusScope::TerminalControl);
    crate::protocol::intent::IntentResult::empty()
}

/// Handles [`Intent::TerminalSendKey`].
pub fn handle_send_key(
    state: &mut AppState,
    bytes: Vec<u8>,
    label: String,
) -> crate::protocol::intent::IntentResult {
    // No-op unless the user actually holds control.
    if state.frontend.scope_stack.current() != &FocusScope::TerminalControl {
        return crate::protocol::intent::IntentResult::empty();
    }
    let _ = label;
    let Some(session_id) = state.frontend.terminal.session_id.clone() else {
        return crate::protocol::intent::IntentResult::empty();
    };
    crate::protocol::intent::IntentResult::empty().with_message(
        crate::feat::interactive_term::protocol::command::SendTermKey {
            session_id: crate::feat::interactive_term::protocol::command::TermSessionId(
                session_id,
            ),
            bytes,
        },
    )
}

/// Handles [`Intent::TerminalHandback`].
///
/// Flips control to the agent, pops to TerminalView, and steers the captured
/// screen to the model. Steering (not a direct message) is the product
/// decision: the queue actor's existing idle/busy handling delivers the
/// screen in both session phases without new wake machinery.
pub fn handle_handback(state: &mut AppState) -> crate::protocol::intent::IntentResult {
    if state.frontend.scope_stack.current() != &FocusScope::TerminalControl {
        return crate::protocol::intent::IntentResult::empty();
    }
    if let Some(flag) = control() {
        flag.set(ControlHolder::Agent);
    }
    state.frontend.terminal.set_control(TermControlHolder::Agent);
    state.frontend.scope_stack.pop();

    // Steer the captured screen to the model (fenced, with instructions).
    let screen = state.frontend.terminal.screen().to_owned();
    let text = format!(
        "The user handed the terminal back to you. Current screen:\n\n\
         ```\n{screen}\n```\n\n{USER_HAS_CONTROL_NOTICE}"
    );
    crate::protocol::intent::IntentResult::empty().with_message(
        crate::feat::chat_input::protocol::command::SubmitSteeringMessage {
            session_id: state.session.active_session_id().clone(),
            text,
        },
    )
}
