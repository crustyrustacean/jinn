//! Attendant built-in tools — `conclude` and `notify_parent`.
//!
//! Two direction-named, single-purpose tools. `report` records what the
//! attendant concluded, for the user; `notify_parent` starts a turn in the
//! parent. There is deliberately no tool that *reads* another attendant's
//! reports: a panel of judges stays independent because none can see the
//! others' conclusions.

use crate::tool_types::ToolContext;
use jinn_attendant_msg::AttendantTrigger;
use jinn_chat_input_msg::EnqueueUserMessage;
use jinn_core_types::chat_entry::ChatEntry;
use jinn_core_types::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_session_msg::SessionOrigin;
use jinn_session_store_msg::PersistSession;

use super::BoxedToolFuture;

/// The `conclude` tool definition: append to the calling attendant's log.
///
/// Named for the act rather than the artifact. A run concludes once, which
/// is why the name is terminal: it rules out calling this repeatedly to
/// build a wall of text, without spending a line of description saying so.
pub fn conclude_definition() -> ToolDefinition {
    ToolDefinition {
        name: "conclude".to_owned(),
        description: "Record what you concluded during your execution. Information saved here will be made available to you later."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![
            "Use `conclude` to leave your concise verdict or findings. The first line should be up to 10 words for human consumption, and the remaining lines are context for you to use later."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "body": {
                    "type": "string",
                    "description": "Your conclusion. Keep it brief."
                }
            },
            "required": ["body"]
        }),
        server_tool_type: None,
    }
}

/// The `notify_parent` tool definition: start a turn in the parent session.
pub fn notify_parent_definition() -> ToolDefinition {
    ToolDefinition {
        name: "notify_parent".to_owned(),
        description: "Send a message to the parent session."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![
            "Call `notify_parent` to send a message to the parent session. This should only be called if your attending rules indicate that you are supposed to interact with your parent."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message the parent session receives."
                }
            },
            "required": ["message"]
        }),
        server_tool_type: None,
    }
}

/// Executes `report`: appends to the calling session's own report log.
///
/// Terminal by design — no message reaches the parent. The append happens
/// under the state write lock; the follow-up persist is published on the bus.
pub fn conclude_execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(body) = argument(&call, "body") else {
            return missing_argument(call, "body");
        };
        let Some(state) = ctx.state.clone() else {
            return failed(call, "conclude is unavailable without shared state");
        };
        let Some(session_id) = ctx.session_id.clone() else {
            return failed(call, "conclude is unavailable without a session context");
        };

        {
            let mut guard = state.write();
            let Some(session) = guard.session.get_mut(&session_id) else {
                return failed(call, "calling session is not live");
            };
            session.append_attendant_report(body);
        }

        // Persist so the conclusion survives the process.
        if let Some(bus) = ctx.bus.clone() {
            bus.publish(PersistSession { session_id }).await;
        }

        succeeded(call, "Conclusion recorded.")
    })
}

/// Executes `notify_parent`: enqueues a user message into the parent session.
///
/// Marks the parent interacted — a programmatic enqueue does not, and without
/// the mark `is_persistable()` gates every save, so the parent's turn would
/// run and silently never reach disk. Also marks the parent's turn automated,
/// so its completion does not fire its own attendants (the suppression that
/// keeps a notify loop from spinning unattended).
pub fn notify_parent_execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(message) = argument(&call, "message") else {
            return missing_argument(call, "message");
        };
        let Some(state) = ctx.state.clone() else {
            return failed(call, "notify_parent is unavailable without shared state");
        };
        let Some(session_id) = ctx.session_id.clone() else {
            return failed(
                call,
                "notify_parent is unavailable without a session context",
            );
        };
        let Some(bus) = ctx.bus.clone() else {
            return failed(call, "notify_parent is unavailable without a bus");
        };

        let (parent_id, wake) = {
            let guard = state.read();
            let Some(caller) = guard.session.get(&session_id) else {
                return failed(call, "calling session is not live");
            };
            // Only an attendant may notify a parent, and there is no way to
            // name a target: the direction is fixed by the caller's own link.
            if caller.origin() != SessionOrigin::Attendant {
                return failed(call, "only an attendant session can notify a parent");
            }
            let Some(parent_id) = caller.parent_session().clone() else {
                return failed(call, "calling attendant has no parent session");
            };
            (
                parent_id,
                caller.attendant_trigger() == AttendantTrigger::ParentCompleted,
            )
        };

        // Mutate the parent: mark interacted (persistence gate) and mark the
        // turn automated (self-retrigger suppression).
        {
            let mut guard = state.write();
            let Some(parent) = guard.session.get_mut(&parent_id) else {
                return failed(call, "parent session is not live");
            };
            parent.mark_interacted();
            if wake {
                parent.mark_turn_automated();
            }
        }

        // Wake the parent with a normal user turn. A `User`-kind entry is
        // deliberate: a non-User entry pushed through the Idle enqueue path
        // would set an empty-string title on an untitled session, and the
        // title guard would then block a real title forever.
        bus.publish(EnqueueUserMessage {
            session_id: parent_id,
            entry: ChatEntry::user(message),
        })
        .await;

        succeeded(call, "Parent session notified.")
    })
}

/// Reads one string argument from the call's JSON arguments.
fn argument(call: &ToolCall, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// Builds a failed result with a short reason for the model.
fn failed(call: ToolCall, reason: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content: format!("Error: {reason}"),
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

/// Builds a failed result for a missing argument, naming the schema slot.
fn missing_argument(call: ToolCall, key: &str) -> ToolResult {
    failed(call, &format!("missing required argument: {key}"))
}

/// Builds a successful result.
fn succeeded(call: ToolCall, content: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content: content.to_owned(),
        success: true,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}
