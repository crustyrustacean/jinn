//! Typed command dispatch — WIT `Command` variant → domain `BridgeClosure`.
//!
//! This is the WASM replacement for the old Lua `dispatch_verb` (see
//! `crates/jinn-domain/src/common/plugin_bridge.rs`). The Lua system translated
//! a *string* verb + *JSON* payload into a typed domain message by matching
//! against each message type's `VERB` constant and deserializing the JSON.
//!
//! Under WASM the boundary is already typed: the plugin emits a WIT
//! `command` variant whose every arm carries a typed record. So there is
//! nothing to match on and nothing to deserialize — each arm of this single
//! `match` constructs the domain message directly and publishes it as a
//! [`BridgeClosure`].
//!
//! Lives in `jinn-wasm-host` (not `jinn-domain`) because it references the
//! generated WIT `Command` type; `jinn-domain` must not depend on the host
//! crate (that would cycle). The host crate already depends on `jinn-domain`.

use error_stack::Report;

use jinn_domain::common::bridge::{Bridge, BridgeClosure};
use jinn_domain::common::bus::BusMessage;

use crate::bindings::command::Command;
use crate::store::InstanceCtx;

/// Error translating a WIT `Command` into a domain `BridgeClosure`.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct CommandDispatchError;

/// Translate a typed WIT `Command` into a `BridgeClosure` ready for the bus.
///
/// Each arm builds the concrete domain message and wraps it in
/// [`Bridge::publish_closure`], which monomorphizes per message type and erases
/// the type at the closure boundary — exactly where the kameo bus needs it (it
/// routes by `TypeId::of::<M>()`).
///
/// `plugin_name` is attached for error attribution only.
///
/// # Errors
///
/// Returns an error only if a `session-id`/`instance-id` field is structurally
/// invalid (should not happen for a well-formed component).
pub fn dispatch(
    plugin_name: &str,
    ctx: &InstanceCtx,
    command: Command,
) -> Result<Option<BridgeClosure>, Report<CommandDispatchError>> {
    let closure = match command {
        Command::PushChatEntry(c) => {
            let entry = match c.kind {
                crate::bindings::command::PushEntryKind::System(t) => {
                    jinn_domain::protocol::ChatEntry::system(t)
                }
                crate::bindings::command::PushEntryKind::Transient(t) => {
                    jinn_domain::protocol::ChatEntry::transient(t)
                }
                crate::bindings::command::PushEntryKind::Error(t) => {
                    jinn_domain::protocol::ChatEntry::error(t)
                }
            };
            publish(
                plugin_name,
                jinn_domain::feat::chat_input::protocol::command::PushChatEntry {
                    session_id: sid(c.session_id)?,
                    entry,
                },
            )
        }
        Command::EnqueueUserMessage(c) => publish(
            plugin_name,
            jinn_domain::feat::chat_input::protocol::command::EnqueueUserMessage {
                session_id: sid(c.session_id)?,
                entry: jinn_domain::protocol::ChatEntry::user(c.text),
            },
        ),
        Command::SetChatInput(c) => publish(
            plugin_name,
            jinn_domain::feat::chat_input::protocol::command::SetChatInputText {
                session_id: sid(c.session_id)?,
                text: c.text,
            },
        ),
        Command::SetChatInputEnabled(c) => publish(
            plugin_name,
            jinn_domain::feat::chat_input::protocol::command::SetChatInputEnabled {
                session_id: sid(c.session_id)?,
                enabled: c.enabled,
            },
        ),
        Command::DisablePlugin(c) => publish(
            plugin_name,
            jinn_domain::feat::plugin_dispatch::protocol::command::TogglePlugin {
                session_id: sid(c.session_id)?,
                plugin_name: c.plugin_name,
                instance_id: instance(c.instance_id)?,
            },
        ),
        Command::EnablePlugin(c) => publish(
            plugin_name,
            jinn_domain::feat::plugin_dispatch::protocol::command::EnablePlugin {
                session_id: sid(c.session_id)?,
                plugin_name: c.plugin_name,
                instance_id: instance(c.instance_id)?,
            },
        ),
        Command::SetManagedSession(c) => publish(
            plugin_name,
            jinn_domain::feat::plugin_dispatch::protocol::command::SetManagedSession {
                session_id: sid(c.session_id)?,
                plugin_name: c.plugin_name,
                managed_session_id: sid(c.managed_session_id)?,
                instance_id: instance(c.instance_id)?,
            },
        ),
        Command::ResetSession(c) => publish(
            plugin_name,
            jinn_domain::feat::session::protocol::reset_session_history::ResetSessionHistory {
                session_id: sid(c.session_id)?,
            },
        ),
        Command::FireAsyncHook(c) => publish(
            plugin_name,
            jinn_domain::common::actor::protocol::dynamic_command::DynamicCommand {
                name: "plugin::fire_async".to_owned(),
                payload: serde_json::json!({
                    "hook": c.hook,
                    "session_id": c.session_id,
                    "text": c.text,
                }),
            },
        ),
    };

    let _ = ctx;
    Ok(Some(closure))
}

/// Wrap a domain `BusMessage` in a publish closure. The message is type-erased
/// at the closure boundary, exactly where the bus needs it.
fn publish<M: BusMessage>(plugin_name: &str, msg: M) -> BridgeClosure {
    tracing::trace!(plugin = %plugin_name, "dispatching plugin command");
    Bridge::publish_closure(msg)
}

/// A WIT `session-id` (a string) is already a valid domain [`SessionId`].
fn sid(s: String) -> Result<jinn_domain::protocol::SessionId, Report<CommandDispatchError>> {
    Ok(s.into())
}

/// A WIT `instance-id` (a string) is already a valid domain [`PluginInstanceId`].
fn instance(s: String) -> Result<jinn_core_types::PluginInstanceId, Report<CommandDispatchError>> {
    Ok(s.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use crate::bindings::command::{
        Command, CreateSessionReq, CreateSessionResp, LlmOneshotReq, LlmResp, RequestError,
    };
    use crate::store::InstanceCtx;
    use jinn_core_types::PluginInstanceId;

    fn ctx() -> InstanceCtx {
        InstanceCtx {
            plugin_name: "test".to_owned(),
            instance_id: PluginInstanceId::new(),
            session_id: None,
        }
    }

    #[test]
    fn push_chat_entry_system_dispatches_to_closure() {
        // Given a typed PushChatEntry(system) command.
        let cmd = Command::PushChatEntry(crate::bindings::command::PushChatEntryCmd {
            session_id: "s-1".to_owned(),
            kind: crate::bindings::command::PushEntryKind::System("hi".to_owned()),
        });

        // When dispatching.
        let result = dispatch("test", &ctx(), cmd);

        // Then a closure is produced.
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn enqueue_user_message_dispatches() {
        let cmd = Command::EnqueueUserMessage(crate::bindings::command::EnqueueUserMessageCmd {
            session_id: "s-1".to_owned(),
            text: "hello".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn set_chat_input_dispatches() {
        let cmd = Command::SetChatInput(crate::bindings::command::SetChatInputCmd {
            session_id: "s-1".to_owned(),
            text: "hello".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn set_chat_input_enabled_dispatches() {
        let cmd = Command::SetChatInputEnabled(crate::bindings::command::SetChatInputEnabledCmd {
            session_id: "s-1".to_owned(),
            enabled: true,
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn disable_plugin_dispatches() {
        let cmd = Command::DisablePlugin(crate::bindings::command::DisablePluginCmd {
            session_id: "s-1".to_owned(),
            plugin_name: "p".to_owned(),
            instance_id: "i-1".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn enable_plugin_dispatches() {
        let cmd = Command::EnablePlugin(crate::bindings::command::EnablePluginCmd {
            session_id: "s-1".to_owned(),
            plugin_name: "p".to_owned(),
            instance_id: "i-1".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn set_managed_session_dispatches() {
        let cmd = Command::SetManagedSession(crate::bindings::command::SetManagedSessionCmd {
            session_id: "s-1".to_owned(),
            plugin_name: "p".to_owned(),
            managed_session_id: "s-2".to_owned(),
            instance_id: "i-1".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn reset_session_dispatches() {
        let cmd = Command::ResetSession(crate::bindings::command::ResetSessionCmd {
            session_id: "s-1".to_owned(),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    #[test]
    fn fire_async_hook_dispatches() {
        let cmd = Command::FireAsyncHook(crate::bindings::command::FireAsyncHookCmd {
            session_id: "s-1".to_owned(),
            hook: "on_enrich".to_owned(),
            text: Some("hello".to_owned()),
        });
        assert!(dispatch("test", &ctx(), cmd).unwrap().is_some());
    }

    // Verify the CreateSession/LlmOneshot request types are usable (they are
    // referenced by the host-import callbacks, not dispatch, but compiling
    // here guards against WIT drift).
    #[test]
    fn request_types_exist() {
        let _ = LlmOneshotReq {
            session_id: "s-1".to_owned(),
            system: "sys".to_owned(),
            prompt: "p".to_owned(),
            persist: false,
            disable_tool_loop: false,
            timeout_ms: Some(30_000),
            task: None,
        };
        let _ = CreateSessionReq {
            parent_session_id: "s-1".to_owned(),
            automated: false,
            persist: true,
            inherit_tools: false,
            tools: vec![],
        };
        let _ = (
            LlmResp {
                text: String::new(),
            },
            CreateSessionResp {
                session_id: "s-2".to_owned(),
            },
        );
        let _ = RequestError::Cancelled;
    }
}
