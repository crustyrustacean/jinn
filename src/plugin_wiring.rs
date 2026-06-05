//! Plugin wiring — command dispatcher.
//!
//! This file is the **only place** that maps plugin command names (strings)
//! to typed domain Commands. Plugins call `ctx.emit("command_name", { ... })`
//! and the dispatcher matches on the name.
//!
//! To add a new plugin command:
//! 1. Add a match arm in [`handle_plugin_command`]
//! 2. Update the plugin's Lua script to use the new command name

use std::sync::Arc;

use jinn_domain::Command;
use jinn_domain::common::actor::message_sink::MessageSink;
use jinn_domain::feat::chat_input::protocol::command::{EnqueueUserMessage, PushChatEntry};
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_domain::protocol::SessionId;
use jinn_plugin::PluginCommand;

/// Dispatch a plugin command to the appropriate domain action.
///
/// Matches on `cmd.name` and sends the corresponding domain `Command`
/// through the provided sink. Unknown commands are logged and dropped.
pub fn handle_plugin_command(cmd: PluginCommand, sink: &dyn MessageSink) {
    tracing::info!(
        name = cmd.name,
        "DIAG plugin-wiring: dispatching plugin command"
    );
    if let Some(domain_cmd) = translate_command(&cmd) {
        tracing::info!(cmd = %domain_cmd, "DIAG plugin-wiring: sending to sink");
        let _ = sink.send_command(domain_cmd);
    } else {
        tracing::info!(
            name = cmd.name,
            "DIAG plugin-wiring: command translated to None"
        );
    }
}

fn translate_command(cmd: &PluginCommand) -> Option<Command> {
    match cmd.name.as_str() {
        "push_chat_entry" => translate_push_chat_entry(cmd, ChatEntry::system),
        "push_chat_entry_transient" => translate_push_chat_entry(cmd, ChatEntry::transient),
        "enqueue_user_message" => translate_enqueue_user_message(cmd),
        _ => {
            tracing::warn!(name = cmd.name, "unknown plugin command");
            None
        }
    }
}

/// Translates a `push_chat_entry` payload into `Command::PushChatEntry`.
fn translate_push_chat_entry(
    cmd: &PluginCommand,
    make_entry: fn(String) -> ChatEntry,
) -> Option<Command> {
    let session_id_str = cmd.data.get("session_id")?.as_str()?;
    let message = cmd
        .data
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Welcome to jinn!")
        .to_owned();

    let entry = make_entry(message);
    Some(Command::PushChatEntry(PushChatEntry {
        session_id: SessionId::from(session_id_str.to_owned()),
        entry,
    }))
}

/// Translates an `enqueue_user_message` payload into `Command::EnqueueUserMessage`.
///
/// This is the command that actually dispatches to the LLM — the fix for
/// `judge_fail` which previously used `push_user` (insert-only, no dispatch).
fn translate_enqueue_user_message(cmd: &PluginCommand) -> Option<Command> {
    let session_id_str = cmd.data.get("session_id")?.as_str()?;
    let text = cmd.data.get("text").and_then(|v| v.as_str())?.to_owned();

    Some(Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id: SessionId::from(session_id_str.to_owned()),
        entry: ChatEntry::user(text),
    }))
}

/// Handle a request from an async hook's `ctx.request(name, data)` call.
///
/// Returns a JSON response value. Unknown requests return null.
#[must_use]
pub fn handle_plugin_request(name: &str, _data: &serde_json::Value) -> serde_json::Value {
    match name {
        "llm" => {
            // LLM requests are handled by the workflow controller, not here.
            // For now, return null — the plugin system's request handler
            // will be wired to the domain LLM service in a future phase.
            tracing::warn!(name, "llm request handler not yet wired");
            serde_json::Value::Null
        }
        _ => {
            tracing::warn!(name, "unknown plugin request");
            serde_json::Value::Null
        }
    }
}

/// Build a command dispatcher closure suitable for `PluginSystem::new`.
///
/// The closure receives `PluginCommand`s and dispatches them through
/// the provided sink.
#[must_use]
pub fn build_command_dispatcher(
    sink: Arc<dyn MessageSink>,
) -> Arc<dyn Fn(PluginCommand) + Send + Sync> {
    Arc::new(move |cmd: PluginCommand| {
        handle_plugin_command(cmd, sink.as_ref());
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]

    use super::*;
    use jinn_domain::Event;
    use jinn_domain::common::actor::actor_ref::SendResult;
    use jinn_domain::common::actor::message_sink::MessageSink;
    use jinn_plugin::PluginCommand;
    use std::sync::{Arc, Mutex};

    /// A mock message sink that captures commands for inspection.
    struct CapturingSink {
        commands: Mutex<Vec<Command>>,
    }

    impl MessageSink for CapturingSink {
        fn name(&self) -> &'static str {
            "test-sink"
        }

        fn send_command(&self, command: Command) -> SendResult {
            self.commands.lock().expect("lock").push(command);
            Ok(())
        }

        fn send_event(&self, _event: Event) -> SendResult {
            Ok(())
        }
    }

    fn test_sink() -> Arc<CapturingSink> {
        Arc::new(CapturingSink {
            commands: Mutex::new(Vec::new()),
        })
    }

    fn captured(sink: &CapturingSink) -> Vec<Command> {
        sink.commands.lock().expect("lock").clone()
    }

    #[test]
    fn push_chat_entry_dispatches_system_entry() {
        let sink = test_sink();
        let cmd = PluginCommand {
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "message": "Hello from plugin!",
            }),
        };

        handle_plugin_command(cmd, &*sink);
        let cmds = captured(&sink);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Command::PushChatEntry(pce) => {
                assert_eq!(pce.session_id, SessionId::from("test-session".to_owned()));
                assert!(pce.entry.text().contains("Hello from plugin!"));
            }
            other => panic!("expected PushChatEntry, got {other:?}"),
        }
    }

    #[test]
    fn enqueue_user_message_dispatches() {
        let sink = test_sink();
        let cmd = PluginCommand {
            name: "enqueue_user_message".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "text": "retry the judgment",
            }),
        };

        handle_plugin_command(cmd, &*sink);
        let cmds = captured(&sink);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Command::EnqueueUserMessage(msg) => {
                assert_eq!(msg.session_id, SessionId::from("test-session".to_owned()));
                assert!(msg.entry.text().contains("retry the judgment"));
            }
            other => panic!("expected EnqueueUserMessage, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_is_dropped() {
        let sink = test_sink();
        let cmd = PluginCommand {
            name: "nonexistent".to_owned(),
            data: serde_json::json!({}),
        };

        handle_plugin_command(cmd, &*sink);
        assert!(captured(&sink).is_empty());
    }

    #[test]
    fn missing_session_id_is_dropped() {
        let sink = test_sink();
        let cmd = PluginCommand {
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({ "message": "hello" }),
        };

        handle_plugin_command(cmd, &*sink);
        assert!(captured(&sink).is_empty());
    }
}
