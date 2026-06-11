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
//FIXME: disabled during actor migration — plugin wiring needs redesign for typed bus messages
// use jinn_domain::Command;
// use jinn_domain::common::actor::message_sink::MessageSink;
use jinn_domain::common::actor::protocol::dynamic_command::DynamicCommand;
use jinn_domain::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use jinn_domain::feat::plugin_dispatch::DomainNodeContext;
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

#[derive(serde::Deserialize)]
struct LuaFireAsyncHook {
    /// Name of the async hook to fire on the plugin-async VM.
    hook: String,
    session_id: SessionId,
    /// Optional extra context (e.g. the input text being enriched).
    #[serde(default)]
    text: Option<String>,
}

#[derive(serde::Deserialize)]
struct LuaSetChatInput {
    session_id: SessionId,
    text: String,
}

//FIXME: disabled during actor migration — verb→Command conversions need redesign for typed bus messages
#[cfg(feature = "disabled-during-migration")]
mod verb_translations {
    use super::*;

    pub(super) fn push_chat_entry_from_lua(
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

    pub(super) fn enqueue_user_message_from_lua(
        _ctx: CmdCtx,
        lua: LuaEnqueueUserMessage,
    ) -> Result<Command, Report<PluginWiringError>> {
        Ok(Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: lua.session_id,
            entry: ChatEntry::user(lua.text),
        }))
    }

    pub(super) fn disable_plugin_from_lua(
        _ctx: CmdCtx,
        lua: LuaDisablePlugin,
    ) -> Result<Command, Report<PluginWiringError>> {
        Ok(Command::TogglePlugin(TogglePlugin {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
        }))
    }

    /// Generic async handoff: routes an arbitrary hook name to the async VM via
    /// the existing `Command::Dynamic` bus path.
    pub(super) fn fire_async_hook_from_lua(
        _ctx: CmdCtx,
        lua: LuaFireAsyncHook,
    ) -> Result<Command, Report<PluginWiringError>> {
        let payload = serde_json::json!({
            "hook": lua.hook,
            "session_id": lua.session_id.to_string(),
            "text": lua.text,
        });
        Ok(Command::Dynamic(DynamicCommand {
            name: "plugin::fire_async".to_owned(),
            payload,
        }))
    }

    pub(super) fn set_chat_input_from_lua(
        _ctx: CmdCtx,
        lua: LuaSetChatInput,
    ) -> Result<Command, Report<PluginWiringError>> {
        Ok(Command::SetChatInputText(SetChatInputText {
            session_id: lua.session_id,
            text: lua.text,
        }))
    }

    pub(super) fn translate<LuaT>(
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

    pub(super) fn translate_command(
        cmd: &PluginCommand,
    ) -> Result<Command, Report<PluginWiringError>> {
        match cmd.name.as_str() {
            "push_chat_entry" => translate::<LuaPushChatEntry>(cmd, push_chat_entry_from_lua),
            "enqueue_user_message" => {
                translate::<LuaEnqueueUserMessage>(cmd, enqueue_user_message_from_lua)
            }
            "disable_plugin" => translate::<LuaDisablePlugin>(cmd, disable_plugin_from_lua),
            "fire_async_hook" => translate::<LuaFireAsyncHook>(cmd, fire_async_hook_from_lua),
            "set_chat_input" => translate::<LuaSetChatInput>(cmd, set_chat_input_from_lua),
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
}


/// Handle a request from an async hook's `ctx.request(name, data)` call.
///
/// Returns a JSON response value. Unknown requests return null.
/// Handle a request from an async hook's `ctx.request(name, data)` call.
///
/// Returns a result envelope: `{ ok: true, value }` on success, or
/// `{ ok: false, error }` on any failure (LLM error, malformed payload,
/// unknown request name).
pub async fn handle_plugin_request(
    name: &str,
    data: &serde_json::Value,
    domain_ctx: &DomainNodeContext,
) -> serde_json::Value {
    match name {
        "llm_oneshot" => {
            // History-less one-shot LLM request: inherits only the source session's
            // provider+model. Request shape:
            //   { session_id, system: Option<String>, prompt: String, persist: Option<bool> }
            // persist defaults to false — one-shots are transient unless the caller
            // explicitly asks to keep them (e.g. a judge run).
            #[derive(serde::Deserialize)]
            struct LlmOneshotPayload {
                session_id: SessionId,
                system: Option<String>,
                prompt: String,
                #[serde(default)]
                persist: Option<bool>,
                // Whether the one-shot session is immune to tool-call loops.
                // true  -> empty tool definitions + tool_loop_disabled set
                // false -> inherit the full global tool catalog (default)
                #[serde(default)]
                disable_tool_loop: Option<bool>,
                // Hard timeout for the one-shot in milliseconds.
                // On expiry the underlying session is hard-cancelled (CancelStream)
                // and the await returns an error. Defaults to 30000.
                #[serde(default)]
                timeout_ms: Option<u64>,
            }
            match serde_json::from_value::<LlmOneshotPayload>(data.clone()) {
                Ok(p) => match domain_ctx
                    .send_llm_request_oneshot(
                        &p.session_id,
                        p.prompt,
                        p.system,
                        p.persist.unwrap_or(false),
                        p.disable_tool_loop.unwrap_or(false),
                        p.timeout_ms.unwrap_or(30_000),
                    )
                    .await
                {
                    Ok(text) => request_ok(serde_json::json!({ "text": text })),
                    Err(e) => {
                        tracing::warn!(error = %e, "llm_oneshot request failed");
                        request_err(format_args!("{e:?}"))
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "llm_oneshot malformed payload");
                    request_err(e)
                }
            }
        }
        "llm" => {
            // Full-context LLM (future use): not wired in this phase.
            tracing::warn!(name, "full-context llm request handler not yet wired");
            request_err("full-context llm request handler not yet wired")
        }
        _ => {
            tracing::warn!(name, "unknown plugin request");
            request_err(format_args!("unknown request: {name}"))
        }
    }
}

/// Wrap a success value in the `ctx.request` result envelope.
fn request_ok(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "ok": true, "value": value })
}

/// Wrap an error in the `ctx.request` result envelope.
fn request_err(error: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": error.to_string() })
}

//FIXME: disabled during actor migration — build_command_dispatcher needs redesign for typed bus messages
pub fn build_command_dispatcher(
    _sink: Arc<dyn std::any::Any>,
) -> Arc<dyn Fn(PluginCommand) + Send + Sync> {
    Arc::new(move |cmd: PluginCommand| {
        tracing::debug!(?cmd, "plugin command dispatch disabled during migration");
    })
}

//FIXME: disabled during actor migration — plugin wiring tests need redesign
#[cfg(all(test, feature = "disabled-during-migration"))]

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
    fn push_chat_entry_error_kind_translates() {
        let sink = test_sink();
        let cmd = PluginCommand {
            plugin_name: "test-plugin".to_owned(),
            name: "push_chat_entry".to_owned(),
            data: serde_json::json!({
                "session_id": "test-session",
                "kind": { "error": "enrichment failed" },
            }),
        };
        handle_plugin_command(cmd, &*sink);
        let cmds = captured(&sink);
        assert_eq!(cmds.len(), 1);
        if let Command::PushChatEntry(pce) = &cmds[0] {
            assert_eq!(pce.entry.kind_str(), "error");
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

    #[test]
    fn fire_async_hook_translates_to_dynamic_command() {
        let cmd = PluginCommand {
            plugin_name: "prompt_enrichment".to_owned(),
            name: "fire_async_hook".to_owned(),
            data: serde_json::json!({
                "hook": "on_enrich",
                "session_id": "s-test-session",
                "text": "hello world",
            }),
        };
        let result = translate_command(&cmd).expect("should translate");
        match result {
            Command::Dynamic(d) => {
                assert_eq!(d.name, "plugin::fire_async");
                assert_eq!(d.payload["hook"], "on_enrich");
                assert_eq!(d.payload["session_id"], "s-test-session");
                assert_eq!(d.payload["text"], "hello world");
            }
            other => panic!("expected Dynamic, got {other:?}"),
        }
    }

    #[test]
    fn fire_async_hook_works_without_text() {
        let cmd = PluginCommand {
            plugin_name: "prompt_enrichment".to_owned(),
            name: "fire_async_hook".to_owned(),
            data: serde_json::json!({
                "hook": "on_toggle",
                "session_id": "s-test-session",
            }),
        };
        let result = translate_command(&cmd).expect("should translate");
        let Command::Dynamic(d) = result else {
            panic!("expected Dynamic");
        };
        assert_eq!(d.name, "plugin::fire_async");
        assert_eq!(d.payload["hook"], "on_toggle");
        assert!(d.payload.get("text").is_none_or(|v| v.is_null()));
    }

    #[test]
    fn set_chat_input_translates_to_set_chat_input_text() {
        let cmd = PluginCommand {
            plugin_name: "prompt_enrichment".to_owned(),
            name: "set_chat_input".to_owned(),
            data: serde_json::json!({
                "session_id": "s-test-session",
                "text": "enriched prompt text",
            }),
        };
        let result = translate_command(&cmd).expect("should translate");
        match result {
            Command::SetChatInputText(s) => {
                assert_eq!(s.session_id.to_string(), "s-test-session");
                assert_eq!(s.text, "enriched prompt text");
            }
            other => panic!("expected SetChatInputText, got {other:?}"),
        }
    }
}
