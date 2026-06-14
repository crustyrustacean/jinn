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
