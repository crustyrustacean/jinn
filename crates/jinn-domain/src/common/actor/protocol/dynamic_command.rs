//! Dynamic command from a plugin or external source.

use serde::{Deserialize, Serialize};

/// A command carrying an arbitrary JSON payload, routed by runtime name.
///
/// Used by plugins to emit commands into the bus. If no actor subscribes
/// to the command's [`name`](Self::name), it is silently dropped.
///
/// The bus dispatches on the runtime `name` field (e.g. `"welcome::show"`)
/// rather than a static constant. This allows plugins to define arbitrary
/// routing keys without recompilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicCommand {
    /// Dot-namespaced identifier, e.g. `"welcome::show"`.
    pub name: String,
    /// Arbitrary JSON payload from the plugin.
    pub payload: serde_json::Value,
}

impl crate::common::bus::BusMessage for DynamicCommand {}

impl crate::common::plugin_bridge::TryFromLua for DynamicCommand {
    const VERB: &'static str = "fire_async_hook";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        use error_stack::ResultExt;

        #[derive(Deserialize)]
        struct LuaPayload {
            hook: String,
            session_id: crate::protocol::SessionId,
            #[serde(default)]
            text: Option<String>,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]

    use super::*;
    use crate::common::plugin_bridge::{CmdCtx, TryFromLua};

    fn ctx() -> CmdCtx {
        CmdCtx {
            plugin_name: "test-plugin".to_owned(),
            verb: DynamicCommand::VERB.to_owned(),
        }
    }

    #[test]
    fn fire_async_hook_with_text_translates() {
        // Given a fire_async_hook payload with text.
        let payload = serde_json::json!({
            "hook": "on_enrich",
            "session_id": "s-test-session",
            "text": "hello world",
        });

        // When translating.
        let msg = DynamicCommand::try_from_lua(ctx(), payload).expect("should translate");

        // Then it is routed as plugin::fire_async with hook, session_id, and text.
        assert_eq!(msg.name, "plugin::fire_async");
        assert_eq!(msg.payload["hook"], "on_enrich");
        assert_eq!(msg.payload["session_id"], "s-test-session");
        assert_eq!(msg.payload["text"], "hello world");
    }

    #[test]
    fn fire_async_hook_without_text_translates_to_null_text() {
        // Given a fire_async_hook payload with no text field.
        let payload = serde_json::json!({
            "hook": "on_toggle",
            "session_id": "s-test-session",
        });

        // When translating.
        let msg = DynamicCommand::try_from_lua(ctx(), payload).expect("should translate");

        // Then text is present but null (Option<String> defaulted to None).
        assert_eq!(msg.name, "plugin::fire_async");
        assert_eq!(msg.payload["hook"], "on_toggle");
        assert!(
            msg.payload
                .get("text")
                .is_none_or(serde_json::Value::is_null)
        );
    }
}
