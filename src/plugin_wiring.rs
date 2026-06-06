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

use error_stack::{Report, ResultExt};
use jinn_domain::Command;
use jinn_domain::common::actor::message_sink::MessageSink;
use jinn_domain::feat::chat_input::protocol::command::{EnqueueUserMessage, PushChatEntry};
use jinn_domain::feat::plugin_dispatch::protocol::command::TogglePlugin;
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_domain::protocol::SessionId;
use jinn_plugin::PluginCommand;
use wherror::Error;

/// Plugin wiring error — failure to translate a `PluginCommand` into a typed
/// domain `Command`.
#[derive(Debug, Error)]
#[error(debug)]
pub struct PluginWiringError;

/// Context attached to every plugin command translation for error attribution.
#[derive(Debug, Clone)]
pub struct CmdCtx {
    /// Name of the plugin that emitted the command.
    pub plugin_name: String,
    /// Verb name (e.g. `"push_chat_entry"`).
    pub verb: String,
}

impl std::fmt::Display for CmdCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plugin {:?} verb {:?}", self.plugin_name, self.verb)
    }
}

// ─── Lua-side payload types ─────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LuaChatEntryKind {
    System(String),
    Transient(String),
    Error(String),
}

#[derive(serde::Deserialize)]
struct LuaPushChatEntry {
    session_id: SessionId,
    kind: LuaChatEntryKind,
}

#[derive(serde::Deserialize)]
struct LuaEnqueueUserMessage {
    session_id: SessionId,
    text: String,
}

#[derive(serde::Deserialize)]
struct LuaDisablePlugin {
    session_id: SessionId,
    plugin_name: String,
}

// ─── Verb → Command conversions ──────────────────���───────────────────

fn push_chat_entry_from_lua(
    _ctx: CmdCtx,
    lua: LuaPushChatEntry,
) -> Result<Command, Report<PluginWiringError>> {
    let entry = match lua.kind {
        LuaChatEntryKind::System(text) => ChatEntry::system(text),
        LuaChatEntryKind::Transient(text) => ChatEntry::transient(text),
        LuaChatEntryKind::Error(text) => ChatEntry::error(text),
    };
    Ok(Command::PushChatEntry(PushChatEntry {
        session_id: lua.session_id,
        entry,
    }))
}

fn enqueue_user_message_from_lua(
    _ctx: CmdCtx,
    lua: LuaEnqueueUserMessage,
) -> Result<Command, Report<PluginWiringError>> {
    Ok(Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id: lua.session_id,
        entry: ChatEntry::user(lua.text),
    }))
}

fn disable_plugin_from_lua(
    _ctx: CmdCtx,
    lua: LuaDisablePlugin,
) -> Result<Command, Report<PluginWiringError>> {
    Ok(Command::TogglePlugin(TogglePlugin {
        session_id: lua.session_id,
        plugin_name: lua.plugin_name,
    }))
}

fn translate<LuaT>(
    cmd: &PluginCommand,
    convert: fn(CmdCtx, LuaT) -> Result<Command, Report<PluginWiringError>>,
) -> Result<Command, Report<PluginWiringError>>
where
    LuaT: serde::de::DeserializeOwned,
{
    let ctx = CmdCtx {
        plugin_name: cmd.plugin_name.clone(),
        verb: cmd.name.clone(),
    };
    let lua: LuaT = serde_json::from_value(cmd.data.clone())
        .change_context(PluginWiringError)
        .attach(ctx.clone())
        .attach("deserialize payload")?;
    convert(ctx, lua)
}

// ─── Dispatcher ──────────────────────────────────────────────���───────

/// Dispatch a plugin command to the appropriate domain action.
///
/// Matches on `cmd.name` and sends the corresponding domain `Command`
/// through the provided sink. Unknown commands are logged and dropped.
pub fn handle_plugin_command(cmd: PluginCommand, sink: &dyn MessageSink) {
    tracing::debug!(
        plugin = cmd.plugin_name,
        verb = cmd.name,
        "plugin command dispatched"
    );
    match translate_command(&cmd) {
        Ok(domain_cmd) => {
            if let Err(e) = sink.send_command(domain_cmd) {
                tracing::error!(
                    plugin = cmd.plugin_name,
                    verb = cmd.name,
                    error = %e,
                    "plugin command send failed"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                plugin = cmd.plugin_name,
                verb = cmd.name,
                error = %e,
                "plugin command translation failed"
            );
        }
    }
}

fn translate_command(cmd: &PluginCommand) -> Result<Command, Report<PluginWiringError>> {
    match cmd.name.as_str() {
        "push_chat_entry" => translate::<LuaPushChatEntry>(cmd, push_chat_entry_from_lua),
        "enqueue_user_message" => {
            translate::<LuaEnqueueUserMessage>(cmd, enqueue_user_message_from_lua)
        }
        "disable_plugin" => translate::<LuaDisablePlugin>(cmd, disable_plugin_from_lua),
        other => {
            let ctx = CmdCtx {
                plugin_name: cmd.plugin_name.clone(),
                verb: other.to_owned(),
            };
            tracing::warn!(
                plugin = cmd.plugin_name,
                verb = other,
                "unknown plugin verb"
            );
            Err(Report::new(PluginWiringError)
                .attach(ctx)
                .attach("unknown verb"))
        }
    }
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
            plugin_name: "test-plugin".to_owned(),
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "kind": { "system": "Hello from plugin!" },
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
            plugin_name: "test-plugin".to_owned(),
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
            plugin_name: "test-plugin".to_owned(),
            name: "nonexistent".to_owned(),
            data: serde_json::json!({}),
        };

        handle_plugin_command(cmd, &*sink);
        assert!(captured(&sink).is_empty());
    }

    #[test]
    fn push_chat_entry_transient_kind_translates() {
        let sink = test_sink();
        let cmd = PluginCommand {
            plugin_name: "test-plugin".to_owned(),
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "kind": { "transient": "welcome" },
            }),
        };
        handle_plugin_command(cmd, &*sink);
        let cmds = captured(&sink);
        assert_eq!(cmds.len(), 1);
        if let Command::PushChatEntry(pce) = &cmds[0] {
            assert_eq!(pce.entry.kind_str(), "transient");
        } else {
            panic!("expected PushChatEntry");
        }
    }

    #[test]
    fn push_chat_entry_unknown_kind_returns_error() {
        let cmd = PluginCommand {
            plugin_name: "test-plugin".to_owned(),
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "kind": { "user": "hi" },
            }),
        };
        let result = translate_command(&cmd);
        assert!(result.is_err());
    }

    #[test]
    fn disable_plugin_translates_to_toggle() {
        let sink = test_sink();
        let cmd = PluginCommand {
            plugin_name: "test-plugin".to_owned(),
            name: "disable_plugin".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "plugin_name": "judge_pass",
            }),
        };
        handle_plugin_command(cmd, &*sink);
        let cmds = captured(&sink);
        assert_eq!(cmds.len(), 1);
        if let Command::TogglePlugin(t) = &cmds[0] {
            assert_eq!(t.plugin_name, "judge_pass");
        } else {
            panic!("expected TogglePlugin");
        }
    }

    #[test]
    fn unknown_verb_error_carries_plugin_name_and_verb() {
        let cmd = PluginCommand {
            plugin_name: "judge_fail".to_owned(),
            name: "enqueue_chat_message".to_owned(),
            data: serde_json::json!({}),
        };
        let err = translate_command(&cmd).expect_err("should fail");
        let report_str = format!("{err:?}");
        assert!(
            report_str.contains("judge_fail"),
            "missing plugin_name: {report_str}"
        );
        assert!(
            report_str.contains("enqueue_chat_message"),
            "missing verb: {report_str}"
        );
    }

    #[test]
    fn malformed_payload_returns_serde_error() {
        let cmd = PluginCommand {
            plugin_name: "test-plugin".to_owned(),
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({ "message": "hello" }), // missing session_id, kind
        };
        let err = translate_command(&cmd).expect_err("should fail");
        let report_str = format!("{err:?}");
        assert!(
            report_str.contains("test-plugin"),
            "missing plugin_name: {report_str}"
        );
    }
}
