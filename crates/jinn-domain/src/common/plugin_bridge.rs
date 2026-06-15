//! Plugin bridge — translates Lua plugin payloads into typed domain messages.
//!
//! Each domain message that plugins can emit implements [`TryFromLua`]. The
//! single [`dispatch_verb`] function matches the verb string against each
//! `<T as TryFromLua>::VERB` and calls `try_from_lua` to produce the message,
//! wrapping it in a [`BridgeClosure`] for the kameo bus.
//!
//! To add a new plugin command:
//! 1. Add a `TryFromLua` impl for the domain message type
//! 2. Add the type to the `dispatch!` list in [`dispatch_verb`]
//!
//! Both the plugin emit path (`handle_plugin_command`) and the intent-replacement
//! path (`dispatch_replacement_command`) delegate to [`dispatch_verb`].

use error_stack::Report;
use serde_json::Value;

use crate::common::bridge::{Bridge, BridgeClosure};
use crate::common::bus::BusMessage;

/// Error type for plugin→message translation failures.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct PluginBridgeError;

/// Context attached to every plugin command translation for error attribution.
#[derive(Debug, Clone)]
pub struct CmdCtx {
    /// Name of the plugin that emitted the command (or `"<interception>"`).
    pub plugin_name: String,
    /// Verb name (e.g. `"push_chat_entry"`).
    pub verb: String,
}

impl std::fmt::Display for CmdCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plugin {} verb {}", self.plugin_name, self.verb)
    }
}

/// Convert a Lua plugin payload into a typed domain message.
///
/// Each domain message that plugins can emit implements this trait. The
/// dispatch table in [`dispatch_verb`] matches the verb string against
/// each `<T as TryFromLua>::VERB` and calls `try_from_lua` to produce
/// the message.
pub trait TryFromLua: BusMessage + Sized {
    /// The verb name that this handler responds to (e.g. `"push_chat_entry"`).
    const VERB: &'static str;

    /// Convert the raw JSON payload into the typed domain message.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload fails to deserialize into the
    /// message's expected shape.
    fn try_from_lua(ctx: CmdCtx, data: Value) -> Result<Self, Report<PluginBridgeError>>;
}

/// Dispatch a plugin verb to a bridge closure.
///
/// Returns `None` for unknown verbs. On translation failure, logs the error
/// (with the [`CmdCtx`] attached) and returns `None`. The caller decides how
/// to handle `None` (warn-and-drop for emit, filter-out for replacement).
///
/// Each match arm monomorphizes [`Bridge::publish_closure`] for its concrete
/// message type, so the type is erased at the closure boundary — exactly
/// where the kameo bus needs it (it routes by `TypeId::of::<M>()`).
pub fn dispatch_verb(verb: &str, ctx: CmdCtx, payload: Value) -> Option<BridgeClosure> {
    macro_rules! dispatch {
        ($($t:ty),+ $(,)?) => {
            match verb {
                $(<$t as TryFromLua>::VERB => match <$t>::try_from_lua(ctx, payload) {
                    Ok(msg) => Some(Bridge::publish_closure(msg)),
                    Err(e) => {
                        tracing::error!(error = %e, "plugin verb translation failed");
                        None
                    }
                },)+
                _ => None,
            }
        }
    }

    dispatch!(
        crate::feat::chat_input::protocol::command::PushChatEntry,
        crate::feat::chat_input::protocol::command::EnqueueUserMessage,
        crate::feat::chat_input::protocol::command::SetChatInputText,
        crate::feat::chat_input::protocol::command::SetChatInputEnabled,
        crate::feat::plugin_dispatch::protocol::command::TogglePlugin,
        crate::feat::plugin_dispatch::protocol::command::SetManagedSession,
        crate::feat::session::protocol::ResetSessionHistory,
        crate::common::actor::protocol::dynamic_command::DynamicCommand,
    )
}
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use serde_json::json;

    fn ctx(verb: &str) -> CmdCtx {
        CmdCtx {
            plugin_name: "test-plugin".to_owned(),
            verb: verb.to_owned(),
        }
    }

    #[rstest::rstest]
    #[case::push_chat_entry(
        "push_chat_entry",
        json!({"session_id":"s","kind":{"system":"hi"}})
    )]
    #[case::enqueue_user_message(
        "enqueue_user_message",
        json!({"session_id":"s","text":"hi"})
    )]
    #[case::set_chat_input(
        "set_chat_input",
        json!({"session_id":"s","text":"hi"})
    )]
    #[case::set_chat_input_enabled(
        "set_chat_input_enabled",
        json!({"session_id":"s","enabled":true})
    )]
    #[case::disable_plugin(
        "disable_plugin",
        json!({"session_id":"s","plugin_name":"p"})
    )]
    #[case::set_managed_session(
        "set_managed_session",
        json!({"session_id":"s","plugin_name":"p","managed_session_id":"m"})
    )]
    #[case::reset_session("reset_session", json!({"session_id":"s"}))]
    #[case::fire_async_hook("fire_async_hook", json!({"hook":"h","session_id":"s"}))]
    fn dispatch_verb_returns_some_for_registered_verb(
        #[case] verb: &str,
        #[case] payload: serde_json::Value,
    ) {
        // When dispatching a registered verb with a valid payload.
        let result = dispatch_verb(verb, ctx(verb), payload);

        // Then a closure is produced.
        assert!(result.is_some(), "verb {verb:?} should dispatch");
    }

    #[test]
    fn dispatch_verb_returns_none_for_unknown_verb() {
        // Given an unrecognized verb.
        // When dispatching.
        let result = dispatch_verb("not_a_verb", ctx("not_a_verb"), json!({}));

        // Then no closure is produced.
        assert!(result.is_none());
    }

    #[test]
    fn dispatch_verb_returns_none_for_malformed_payload() {
        // Given a registered verb with a payload of the wrong shape.
        // When dispatching push_chat_entry with a payload missing required fields.
        let result = dispatch_verb(
            "push_chat_entry",
            ctx("push_chat_entry"),
            json!({"message": "totally wrong shape"}),
        );

        // Then no closure is produced (translation failed, logged at error level).
        assert!(result.is_none());
    }
}
