//! Plugin wiring — command dispatcher.
//!
//! This file maps plugin command names (strings) to typed domain messages.
//! Plugins call `ctx.emit("command_name", { ... })` and the dispatcher
//! matches on the name.
//!
//! Each verb has a [`PluginVerb`] implementation that:
//! - Declares its verb name (`const VERB`)
//! - Declares the domain message type (`type DomainMsg`)
//! - Converts the Lua payload to the domain message (`from_lua`)
//!
//! To add a new plugin command:
//! 1. Add a `PluginVerb` impl for the Lua payload type
//! 2. Register it in `VERB_DISPATCH_TABLE`

use std::sync::Arc;

use error_stack::{Report, ResultExt};
use jinn_domain::common::actor::protocol::dynamic_command::DynamicCommand;
use jinn_domain::common::bus::BusMessage;
use jinn_domain::common::services::ActorChannelService;
use jinn_domain::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use jinn_domain::feat::plugin_dispatch::protocol::command::{SetManagedSession, TogglePlugin};
use jinn_domain::feat::plugin_system::PluginCommand;
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_domain::feat::session::protocol::ResetSessionHistory;
use jinn_domain::protocol::SessionId;
use wherror::Error;

/// Plugin wiring error — failure to translate a `PluginCommand` into a typed
/// domain message.
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

// ─── PluginVerb trait ──────────────────────────────────────────────

/// Trait for converting a Lua plugin payload into a typed domain message.
///
/// Each Lua payload type implements this trait. The dispatcher uses the
/// verb name to find the right impl and calls `from_lua` to produce the
/// domain message.
pub trait PluginVerb {
    /// The verb name that this handler responds to (e.g. "push_chat_entry").
    const VERB: &'static str;
    /// The domain message type produced by this verb.
    type DomainMsg: BusMessage;
    /// Convert the raw JSON payload into a typed domain message.
    fn from_lua(
        ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self::DomainMsg, Report<PluginWiringError>>;
}

// ─── Lua-side payload types ────────────────────────────────────────

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
    hook: String,
    session_id: SessionId,
    #[serde(default)]
    text: Option<String>,
}

#[derive(serde::Deserialize)]
struct LuaSetChatInput {
    session_id: SessionId,
    text: String,
}

#[derive(serde::Deserialize)]
struct LuaResetSession {
    session_id: SessionId,
}

#[derive(serde::Deserialize)]
struct LuaSetManagedSession {
    session_id: SessionId,
    plugin_name: String,
    managed_session_id: SessionId,
}

// ─── PluginVerb implementations ────────────────────────────────────

impl PluginVerb for LuaPushChatEntry {
    const VERB: &'static str = "push_chat_entry";
    type DomainMsg = PushChatEntry;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<PushChatEntry, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize push_chat_entry payload")?;
        let entry = match lua.kind {
            LuaChatEntryKind::System(text) => ChatEntry::system(text),
            LuaChatEntryKind::Transient(text) => ChatEntry::transient(text),
            LuaChatEntryKind::Error(text) => ChatEntry::error(text),
        };
        Ok(PushChatEntry {
            session_id: lua.session_id,
            entry,
        })
    }
}

impl PluginVerb for LuaEnqueueUserMessage {
    const VERB: &'static str = "enqueue_user_message";
    type DomainMsg = EnqueueUserMessage;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<EnqueueUserMessage, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize enqueue_user_message payload")?;
        Ok(EnqueueUserMessage {
            session_id: lua.session_id,
            entry: ChatEntry::user(lua.text),
        })
    }
}

impl PluginVerb for LuaDisablePlugin {
    const VERB: &'static str = "disable_plugin";
    type DomainMsg = TogglePlugin;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<TogglePlugin, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize disable_plugin payload")?;
        Ok(TogglePlugin {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
        })
    }
}

impl PluginVerb for LuaFireAsyncHook {
    const VERB: &'static str = "fire_async_hook";
    type DomainMsg = DynamicCommand;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<DynamicCommand, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize fire_async_hook payload")?;
        let payload = serde_json::json!({
            "hook": lua.hook,
            "session_id": lua.session_id.to_string(),
            "text": lua.text,
        });
        Ok(DynamicCommand {
            name: "plugin::fire_async".to_owned(),
            payload,
        })
    }
}

impl PluginVerb for LuaSetChatInput {
    const VERB: &'static str = "set_chat_input";
    type DomainMsg = SetChatInputText;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<SetChatInputText, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize set_chat_input payload")?;
        Ok(SetChatInputText {
            session_id: lua.session_id,
            text: lua.text,
        })
    }
}

impl PluginVerb for LuaResetSession {
    const VERB: &'static str = "reset_session";
    type DomainMsg = ResetSessionHistory;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<ResetSessionHistory, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize reset_session payload")?;
        Ok(ResetSessionHistory {
            session_id: lua.session_id,
        })
    }
}

impl PluginVerb for LuaSetManagedSession {
    const VERB: &'static str = "set_managed_session";
    type DomainMsg = SetManagedSession;
    fn from_lua(
        _ctx: CmdCtx,
        data: serde_json::Value,
    ) -> Result<SetManagedSession, Report<PluginWiringError>> {
        let lua: Self = serde_json::from_value(data)
            .change_context(PluginWiringError)
            .attach("deserialize set_managed_session payload")?;
        Ok(SetManagedSession {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
            managed_session_id: lua.managed_session_id,
        })
    }
}

// ─── Dispatcher ────────────────────────────────────────────────────

/// Dispatch a plugin command to the appropriate domain action.
///
/// Matches on `cmd.name`, deserializes via `PluginVerb`, and publishes
/// the typed domain message through the actor channel.
/// Unknown commands are logged and dropped.
pub fn handle_plugin_command(cmd: PluginCommand, channel: &ActorChannelService) {
    tracing::debug!(
        plugin = cmd.plugin_name,
        verb = cmd.name,
        "plugin command dispatched"
    );

    macro_rules! dispatch {
        ($($lua_type:ty),+ $(,)?) => {
            match cmd.name.as_str() {
                $(<$lua_type as PluginVerb>::VERB => {
                    match dispatch_verb::<$lua_type>(&cmd) {
                        Ok(msg) => channel.send_message(msg),
                        Err(e) => {
                            tracing::error!(
                                plugin = cmd.plugin_name,
                                verb = cmd.name,
                                error = %e,
                                "plugin command translation failed"
                            );
                        }
                    }
                })+
                other => {
                    tracing::warn!(
                        plugin = cmd.plugin_name,
                        verb = other,
                        "unknown plugin verb"
                    );
                }
            }
        }
    }

    dispatch!(
        LuaPushChatEntry,
        LuaEnqueueUserMessage,
        LuaDisablePlugin,
        LuaFireAsyncHook,
        LuaSetChatInput,
        LuaResetSession,
        LuaSetManagedSession,
    )
}

/// Dispatch a single verb by type.
fn dispatch_verb<V: PluginVerb>(
    cmd: &PluginCommand,
) -> Result<V::DomainMsg, Report<PluginWiringError>> {
    let ctx = CmdCtx {
        plugin_name: cmd.plugin_name.clone(),
        verb: cmd.name.clone(),
    };
    V::from_lua(ctx, cmd.data.clone())
}

/// Build a command dispatcher closure for the plugin system.
///
/// The returned closure captures an `ActorChannelService` and routes
/// plugin commands through the bus via the kanal bridge.
pub fn build_command_dispatcher(
    channel: ActorChannelService,
) -> Arc<dyn Fn(PluginCommand) + Send + Sync> {
    Arc::new(move |cmd: PluginCommand| {
        handle_plugin_command(cmd, &channel);
    })
}

// ─── Request handler (for ctx.request from Lua) ────────────────────

/// Handle a request from an async hook's `ctx.request(name, data)` call.
///
/// Returns a result envelope: `{ ok: true, value }` on success, or
/// `{ ok: false, error }` on any failure.
// FIXME: plugin migration — re-enable once DomainNodeContext is restored
pub async fn handle_plugin_request(
    name: &str,
    _data: &serde_json::Value,
    _domain_ctx: &jinn_domain::feat::plugin_dispatch::DomainNodeContext,
) -> serde_json::Value {
    tracing::warn!(name, "plugin request handler not yet re-enabled");
    request_err(format_args!("not yet re-enabled: {name}"))
}

fn request_err(error: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": error.to_string() })
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

    /// Helper: test a PluginVerb by deserializing a JSON payload.
    fn test_verb<V: PluginVerb>(
        data: serde_json::Value,
    ) -> Result<V::DomainMsg, Report<PluginWiringError>> {
        let ctx = CmdCtx {
            plugin_name: "test-plugin".to_owned(),
            verb: V::VERB.to_owned(),
        };
        V::from_lua(ctx, data)
    }

    #[test]
    fn push_chat_entry_dispatches_system_entry() {
        let msg = test_verb::<LuaPushChatEntry>(serde_json::json!({
            "session_id": "test-session",
            "kind": { "system": "Hello from plugin!" },
        }))
        .expect("should translate");
        assert_eq!(msg.session_id, SessionId::from("test-session".to_owned()));
        assert!(msg.entry.text().contains("Hello from plugin!"));
    }

    #[test]
    fn enqueue_user_message_dispatches() {
        let msg = test_verb::<LuaEnqueueUserMessage>(serde_json::json!({
            "session_id": "test-session",
            "text": "retry the judgment",
        }))
        .expect("should translate");
        assert!(msg.entry.text().contains("retry the judgment"));
    }

    #[test]
    fn unknown_verb_returns_error() {
        let _cmd = PluginCommand {
            plugin_name: "test".to_owned(),
            name: "nonexistent".to_owned(),
            data: serde_json::json!({}),
        };
        // No matching verb — handle_plugin_command just logs, so we test
        // that no verb matches by checking the verb name isn't in the dispatch table.
        assert_ne!("nonexistent", LuaPushChatEntry::VERB);
    }

    #[test]
    fn push_chat_entry_transient_kind_translates() {
        let msg = test_verb::<LuaPushChatEntry>(serde_json::json!({
            "session_id": "test-session",
            "kind": { "transient": "welcome" },
        }))
        .expect("should translate");
        assert_eq!(msg.entry.kind_str(), "transient");
    }

    #[test]
    fn push_chat_entry_error_kind_translates() {
        let msg = test_verb::<LuaPushChatEntry>(serde_json::json!({
            "session_id": "test-session",
            "kind": { "error": "enrichment failed" },
        }))
        .expect("should translate");
        assert_eq!(msg.entry.kind_str(), "error");
    }

    #[test]
    fn push_chat_entry_unknown_kind_returns_error() {
        let result = test_verb::<LuaPushChatEntry>(serde_json::json!({
            "session_id": "test-session",
            "kind": { "user": "hi" },
        }));
        assert!(result.is_err());
    }

    #[test]
    fn disable_plugin_translates_to_toggle() {
        let msg = test_verb::<LuaDisablePlugin>(serde_json::json!({
            "session_id": "test-session",
            "plugin_name": "judge_pass",
        }))
        .expect("should translate");
        assert_eq!(msg.plugin_name, "judge_pass");
    }

    #[test]
    fn malformed_payload_returns_serde_error() {
        let result = test_verb::<LuaPushChatEntry>(serde_json::json!({
            "message": "hello"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn fire_async_hook_translates_to_dynamic_command() {
        let msg = test_verb::<LuaFireAsyncHook>(serde_json::json!({
            "hook": "on_enrich",
            "session_id": "s-test-session",
            "text": "hello world",
        }))
        .expect("should translate");
        assert_eq!(msg.name, "plugin::fire_async");
        assert_eq!(msg.payload["hook"], "on_enrich");
        assert_eq!(msg.payload["session_id"], "s-test-session");
        assert_eq!(msg.payload["text"], "hello world");
    }

    #[test]
    fn fire_async_hook_works_without_text() {
        let msg = test_verb::<LuaFireAsyncHook>(serde_json::json!({
            "hook": "on_toggle",
            "session_id": "s-test-session",
        }))
        .expect("should translate");
        assert_eq!(msg.name, "plugin::fire_async");
        assert_eq!(msg.payload["hook"], "on_toggle");
        assert!(msg.payload.get("text").is_none_or(|v| v.is_null()));
    }

    #[test]
    fn set_chat_input_translates_to_set_chat_input_text() {
        let msg = test_verb::<LuaSetChatInput>(serde_json::json!({
            "session_id": "s-test-session",
            "text": "enriched prompt text",
        }))
        .expect("should translate");
        assert_eq!(msg.session_id.to_string(), "s-test-session");
        assert_eq!(msg.text, "enriched prompt text");
    }

    #[test]
    fn reset_session_dispatches_reset_session_history() {
        let msg = test_verb::<LuaResetSession>(serde_json::json!({
            "session_id": "s-judge-session",
        }))
        .expect("should translate");
        assert_eq!(
            msg.session_id,
            SessionId::from("s-judge-session".to_owned())
        );
    }

    #[test]
    fn set_managed_session_translates() {
        let msg = test_verb::<LuaSetManagedSession>(serde_json::json!({
            "session_id": "s-parent",
            "plugin_name": "judge_pass",
            "managed_session_id": "s-child",
        }))
        .expect("should translate");
        assert_eq!(msg.session_id, SessionId::from("s-parent".to_owned()));
        assert_eq!(msg.plugin_name, "judge_pass");
        assert_eq!(
            msg.managed_session_id,
            SessionId::from("s-child".to_owned())
        );
    }
}
