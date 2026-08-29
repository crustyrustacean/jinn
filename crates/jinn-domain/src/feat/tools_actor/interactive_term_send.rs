//! `interactive_term_send` built-in tool — sends input to a live session.
//!
//! All of `text`, `keys`, and `enter` are optional: a call with none of them
//! is a **pure screen sync** (the actor re-drains briefly and returns the
//! current screen without writing anything to the PTY). While the user holds
//! control (takeover), input is refused: the result reports success with the
//! current screen plus the wait notice instructing the model to stop and
//! wait — prompting, not enforcement.
//!
//! Each call blocks until the screen settles, then returns the updated
//! rendered screen.

use std::time::Duration;

use futures::FutureExt;

use crate::feat::interactive_term::protocol::command::{
    SendTermInput, SendTermOutcome, TermScreen, TermSessionId,
};
use crate::feat::interactive_term::pty_session::ExitInfo;
use crate::feat::interactive_term::settle::default_max_wait;
use crate::feat::tools_actor::interactive_term::StreamCtx;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;
use super::interactive_term::{failure_result, parse, success_result};

/// The notice returned when the user holds control. Wording is the product
/// decision: it instructs the model to stop and wait — there is no
/// programmatic enforcement.
pub(crate) const USER_HAS_CONTROL_NOTICE: &str = "NOTE: The user has taken control of this terminal. \
     Stop your current response and wait for the user to finish; \
     they will hand the terminal back with a screen update.";

/// Defensive outer bound on the ask; mirrors the spawn tool.
const ASK_MARGIN: Duration = Duration::from_secs(5);

/// Returns the tool definition for the `interactive_term_send` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "interactive_term_send".to_owned(),
        description: "Send input to a running `interactive_term` session: type text, press named \
            keys, or both — then receive the updated rendered screen. \
            \
            Named keys: \"enter\", \"esc\", \"tab\", \"backspace\", \"delete\", \"up\", \"down\", \
            \"left\", \"right\", \"home\", \"end\", \"pageup\", \"pagedown\", \"ctrl+<letter>\" \
            (e.g. \"ctrl+c\"), \"alt+<key>\", or any single character. \
            \
            Call with NO arguments (just session_id) to re-sync the current screen without \
            sending anything. \
            \
            BLOCKING: returns after the screen output settles. If the user has taken control of \
            the terminal, your input is NOT delivered — the result tells you to stop and wait."
            .to_owned(),
        prompt_snippet: Some(
            "Send text/keys to an interactive_term session and get the updated screen".to_owned(),
        ),
        prompt_guidelines: vec![
            "After typing text, include \"enter\": true or the keys entry \"enter\" — text alone does not submit.".to_owned(),
            "If the result says the user has control, STOP and wait for the user to hand the terminal back.".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The session to send input to (from interactive_term)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type verbatim (optional)"
                },
                "keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Named keys to press in order, e.g. [\"ctrl+c\"], [\"up\", \"enter\"] (optional)"
                },
                "enter": {
                    "type": "boolean",
                    "description": "Press enter after the text/keys (optional, default false)"
                },
                "max_duration_secs": {
                    "type": "number",
                    "description": "Maximum seconds to wait for the screen to settle. Default 3."
                }
            },
            "required": ["session_id"]
        }),
        server_tool_type: None,
    }
}

/// Executes the `interactive_term_send` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    let tool_call_id = call.id;
    let tool_name = call.name;

    let Some(coordinator) = ctx.interactive_term else {
        return super::interactive_term::failure_future(
            &tool_call_id,
            &tool_name,
            "interactive-term coordinator is unavailable (this should only happen in tests)",
        );
    };

    let Some(session_raw) = parse::string_field(&call.arguments, "session_id") else {
        return super::interactive_term::failure_future(
            &tool_call_id,
            &tool_name,
            "missing required `session_id` argument (the id returned by interactive_term)",
        );
    };
    let session_id = TermSessionId(session_raw);

    let text = parse::string_field(&call.arguments, "text");
    let keys = parse::keys_field(&call.arguments, "keys");
    let enter = parse::bool_field(&call.arguments, "enter");
    let max_wait = super::extract_max_duration(&call.arguments)
        .map_or_else(default_max_wait, Duration::from_secs);
    let ask_timeout = max_wait + ASK_MARGIN;

    // Stream context for the settle wait's delta events (watchdog keepalive).
    let stream_ctx = ctx.session_id.clone().map(|session_id| StreamCtx {
        session_id,
        tool_call_id: tool_call_id.clone(),
    });

    async move {
        // `AskRequest` needs `.send()` to become a future; the keepalive
        // pacer publishes heartbeats while the ask blocks on the settle wait.
        let ask_fut = coordinator
            .ask(SendTermInput {
                session_id: session_id.clone(),
                text: text.clone(),
                keys: keys.clone(),
                enter,
                max_wait,
            })
            .send();
        let replied =
            super::interactive_term::with_keepalive(ctx.bus, stream_ctx, ask_timeout, ask_fut)
                .await;

        // `None` = the defensive outer bound elapsed without a reply.
        let Some(replied) = replied else {
            return failure_result(
                &tool_call_id,
                &tool_name,
                &format!(
                    "interactive_term_send timed out after {ask_timeout:?} with no reply from the coordinator."
                ),
            );
        };

        let replied = match replied {
            Ok(replied) => replied,
            Err(_send_err) => {
                return failure_result(
                    &tool_call_id,
                    &tool_name,
                    "interactive-term coordinator is unreachable (mailbox failure).",
                );
            }
        };

        match replied {
            SendTermOutcome::Sent(screen) => success_result(
                &tool_call_id,
                &tool_name,
                &session_id,
                &screen.screen,
                screen.exited.as_ref(),
            ),
            SendTermOutcome::UserHasControl(screen) => {
                let mut result = success_result(
                    &tool_call_id,
                    &tool_name,
                    &session_id,
                    &screen.screen,
                    screen.exited.as_ref(),
                );
                result.content = format!("{USER_HAS_CONTROL_NOTICE}\n\n{}", result.content);
                result
            }
            SendTermOutcome::UnknownSession => failure_result(
                &tool_call_id,
                &tool_name,
                &format!(
                    "unknown session `{session_id}`. Sessions die with the app; \
                     spawn a new one with interactive_term."
                ),
            ),
            SendTermOutcome::Exited(screen) => {
                format_exited(&tool_call_id, &tool_name, &session_id, &screen)
            }
        }
    }
    .boxed()
}

/// Formats the already-exited outcome: exit summary ahead of the screen.
fn format_exited(
    tool_call_id: &str,
    tool_name: &str,
    session_id: &TermSessionId,
    screen: &TermScreen,
) -> ToolResult {
    let code = screen
        .exited
        .as_ref()
        .map_or_else(|| "exited".to_owned(), ExitInfo::summary);
    let mut result = success_result(
        tool_call_id,
        tool_name,
        session_id,
        &screen.screen,
        screen.exited.as_ref(),
    );
    result.content = format!("The session has already {code}.\n\n{}", result.content);
    result
}
