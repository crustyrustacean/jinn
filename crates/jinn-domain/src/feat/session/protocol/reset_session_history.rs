//! Reset a session's chat history.
//!
//! Sent by plugins (via `reset_session` emit verb) to clear a session's
//! conversation history and reset its lifecycle state. Used by judge plugins
//! to start each evaluation with a clean workspace.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Reset a session's chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetSessionHistory {
    /// The session whose history should be cleared.
    pub session_id: SessionId,
}

impl crate::common::bus::BusMessage for ResetSessionHistory {}

impl crate::common::plugin_bridge::TryFromLua for ResetSessionHistory {
    const VERB: &'static str = "reset_session";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        use error_stack::ResultExt;

        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize reset_session payload")?;

        Ok(ResetSessionHistory {
            session_id: lua.session_id,
        })
    }
}
