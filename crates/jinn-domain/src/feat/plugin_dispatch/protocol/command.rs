//! Plugin dispatch protocol — commands.
//!
//! Replaces the workflow command set. Plugins attach to a session by name;
//! the dispatcher loads them into a per-session Lua state and fires their
//! hooks at lifecycle events.

use error_stack::ResultExt;
use serde::{Deserialize, Serialize};

use crate::common::bus::BusMessage;
use crate::feat::attached_plugin::PluginInstanceId;
use crate::protocol::SessionId;

/// Attach an attachable plugin to a session.
///
/// The plugin must exist under `plugins/attachable/`. The dispatcher will:
/// 1. Validate the plugin name.
/// 2. Call `services.plugins.create_session_registry([plugin_name])` to spin up
///    a per-session Lua state.
/// 3. Push `AttachedPlugin { name, enabled: true, run_state: Idle }` onto
///    `session.core.attached_plugins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachPlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// Detach a plugin from a session by name.
///
/// Calls `destroy_session_registry` for the matching registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetachPlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
}

/// Disable an attached plugin instance (force-set `enabled = false`). No-op if
/// not attached.
///
/// Targets a specific plugin *instance* by `instance_id` (required). Two
/// attachments of the same plugin name are distinct instances; this sets the
/// `enabled` flag on exactly the named instance.
///
/// Note: the verb stays `"disable_plugin"` for backward-compat with existing
/// Lua, but the semantics are now force-disable (not a flip).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TogglePlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub instance_id: PluginInstanceId,
}

/// Set the managed session ID on an attached plugin *instance*.
///
/// Called when a plugin creates a child session and wants the sidebar
/// to be able to navigate to it. The activate path reads this field
/// to determine which session to switch to.
///
/// Targets a specific instance by `instance_id` (required) so that duplicate
/// attachments of the same plugin name each hold their own managed session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetManagedSession {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub managed_session_id: SessionId,
    pub instance_id: PluginInstanceId,
}

impl BusMessage for AttachPlugin {}
impl BusMessage for DetachPlugin {}
impl BusMessage for TogglePlugin {}

impl crate::common::plugin_bridge::TryFromLua for TogglePlugin {
    const VERB: &'static str = "disable_plugin";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            plugin_name: String,
            instance_id: PluginInstanceId,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize disable_plugin payload")?;

        Ok(TogglePlugin {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
            instance_id: lua.instance_id,
        })
    }
}
impl BusMessage for SetManagedSession {}

impl crate::common::plugin_bridge::TryFromLua for SetManagedSession {
    const VERB: &'static str = "set_managed_session";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            plugin_name: String,
            managed_session_id: SessionId,
            instance_id: PluginInstanceId,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize set_managed_session payload")?;

        Ok(SetManagedSession {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
            managed_session_id: lua.managed_session_id,
            instance_id: lua.instance_id,
        })
    }
}

/// Enable an attached plugin instance (force-set `enabled = true`). No-op if
/// not attached.
///
/// The symmetric counterpart to [`TogglePlugin`] (which is force-disable).
/// Targets a specific instance by `instance_id` (required). Used by the judge
/// aggregation to re-activate all instances when any judge fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnablePlugin {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub instance_id: PluginInstanceId,
}

impl BusMessage for EnablePlugin {}

impl crate::common::plugin_bridge::TryFromLua for EnablePlugin {
    const VERB: &'static str = "enable_plugin";

    fn try_from_lua(
        ctx: crate::common::plugin_bridge::CmdCtx,
        data: serde_json::Value,
    ) -> Result<Self, error_stack::Report<crate::common::plugin_bridge::PluginBridgeError>> {
        #[derive(Deserialize)]
        struct LuaPayload {
            session_id: SessionId,
            plugin_name: String,
            instance_id: PluginInstanceId,
        }

        let lua: LuaPayload = serde_json::from_value(data)
            .change_context(crate::common::plugin_bridge::PluginBridgeError)
            .attach(ctx)
            .attach("deserialize enable_plugin payload")?;

        Ok(EnablePlugin {
            session_id: lua.session_id,
            plugin_name: lua.plugin_name,
            instance_id: lua.instance_id,
        })
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use crate::common::plugin_bridge::{CmdCtx, TryFromLua};

    fn ctx(verb: &str) -> CmdCtx {
        CmdCtx {
            plugin_name: "test-plugin".to_owned(),
            verb: verb.to_owned(),
        }
    }

    #[test]
    fn disable_plugin_verb_translates_to_toggle_plugin() {
        // Given a disable_plugin payload.
        let payload = serde_json::json!({
            "session_id": "test-session",
            "plugin_name": "judge_pass",
            "instance_id": "i-test-instance",
        });

        // When translating the "disable_plugin" verb.
        let msg =
            TogglePlugin::try_from_lua(ctx(TogglePlugin::VERB), payload).expect("should translate");

        // Then a TogglePlugin message is produced with the plugin name + instance id.
        assert_eq!(msg.plugin_name, "judge_pass");
        assert_eq!(msg.instance_id.to_string(), "i-test-instance");
    }

    #[test]
    fn disable_plugin_verb_const_is_disable_plugin() {
        // Then the verb constant preserves the legacy "disable_plugin" name
        // (intentional: verb name does not match the message name).
        assert_eq!(TogglePlugin::VERB, "disable_plugin");
    }

    #[test]
    fn set_managed_session_translates() {
        // Given a set_managed_session payload.
        let payload = serde_json::json!({
            "session_id": "s-parent",
            "plugin_name": "judge_pass",
            "managed_session_id": "s-child",
            "instance_id": "i-test-instance",
        });

        // When translating.
        let msg = SetManagedSession::try_from_lua(ctx(SetManagedSession::VERB), payload)
            .expect("should translate");

        // Then all fields are preserved (including the instance id).
        assert_eq!(msg.session_id, SessionId::from("s-parent".to_owned()));
        assert_eq!(msg.plugin_name, "judge_pass");
        assert_eq!(
            msg.managed_session_id,
            SessionId::from("s-child".to_owned())
        );
        assert_eq!(msg.instance_id.to_string(), "i-test-instance");
    }
}
