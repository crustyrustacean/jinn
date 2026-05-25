//! Preflight hook infrastructure.
//!
//! Plugins can register Lua callbacks that run before a command is processed.
//! A preflight hook returns `true` (allow) or `false` (veto). If any hook
//! vetoes, the command is not processed.

use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Function, Lua, Value};
use parking_lot::Mutex;

/// Internal preflight hook map stored in Lua app_data.
pub(crate) type PreflightMap = Arc<Mutex<HashMap<String, Vec<Function>>>>;

/// Installs the preflight map into Lua app_data.
///
/// Called once during host initialization. Lua plugins don't have direct
/// access to register preflight hooks yet — that API will be added in a
/// future phase.
pub fn init(lua: &Lua) -> PreflightMap {
    #[expect(clippy::arc_with_non_send_sync, reason = "mlua::Function is not Send but only used from the single Lua thread")]
    let map: PreflightMap = Arc::new(Mutex::new(HashMap::new()));
    lua.set_app_data(map.clone());
    map
}

/// Registers a preflight hook for a command name.
///
/// The hook is a Lua function that receives `(command_name, payload)` and
/// returns `true` (allow) or `false` (veto).
#[expect(clippy::allow_attributes, reason = "cannot use #[expect(dead_code)] because it's unfulfilled in test builds")]
#[allow(dead_code, reason = "scaffold for future Lua preflight registration")]
pub fn register(lua: &Lua, command_name: String, callback: Function) {
    let Some(guard) = lua.app_data_ref::<PreflightMap>() else {
        tracing::warn!("preflight map not found in Lua app_data");
        return;
    };
    guard.lock().entry(command_name).or_default().push(callback);
}

/// Dispatches a preflight check for a command.
///
/// Runs all registered hooks for `command_name`. Returns `true` if all hooks
/// approve (or no hooks are registered). Returns `false` on the first veto.
/// Lua errors during hook execution are logged and treated as vetoes.
pub fn dispatch(lua: &Lua, command_name: &str, payload: &serde_json::Value) -> bool {
    let Some(guard) = lua.app_data_ref::<PreflightMap>() else {
        return true; // No map → allow.
    };

    let callbacks: Vec<Function> = {
        let map = guard.lock();
        map.get(command_name).cloned().unwrap_or_default()
    };

    if callbacks.is_empty() {
        return true;
    }

    for callback in &callbacks {
        let lua_payload = crate::bindings::json_to_lua_value(lua, payload).unwrap_or(Value::Nil);
        match callback.call::<bool>((command_name, lua_payload)) {
            Ok(true) => {},
            Ok(false) => return false,
            Err(e) => {
                tracing::warn!(
                    err = %e,
                    command_name,
                    "preflight hook error, treating as veto"
                );
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]

    use super::*;

    fn test_lua() -> Lua {
        let lua = Lua::new();
        init(&lua);
        lua
    }

    #[rstest::rstest]
    fn preflight_returns_true_when_no_hooks_registered() {
        // Given a Lua VM with no preflight hooks.
        let lua = test_lua();

        // When dispatching preflight for any command.
        let result = dispatch(&lua, "anything", &serde_json::Value::Null);

        // Then it returns true (proceed).
        assert!(result, "should proceed when no hooks registered");
    }

    #[rstest::rstest]
    fn preflight_returns_true_when_hook_approves() {
        // Given a Lua VM with an approving hook.
        let lua = test_lua();
        let callback = lua
            .create_function(|_, _args: (String, Value)| Ok(true))
            .expect("create callback");
        register(&lua, "test::cmd".to_owned(), callback);

        // When dispatching preflight for that command.
        let result = dispatch(&lua, "test::cmd", &serde_json::Value::Null);

        // Then it returns true.
        assert!(result, "should proceed when hook approves");
    }

    #[rstest::rstest]
    fn preflight_returns_false_when_hook_vetoes() {
        // Given a Lua VM with a veto hook.
        let lua = test_lua();
        let callback = lua
            .create_function(|_, _args: (String, Value)| Ok(false))
            .expect("create callback");
        register(&lua, "test::cmd".to_owned(), callback);

        // When dispatching preflight for that command.
        let result = dispatch(&lua, "test::cmd", &serde_json::Value::Null);

        // Then it returns false.
        assert!(!result, "should block when hook vetoes");
    }

    #[rstest::rstest]
    fn preflight_stops_on_first_veto() {
        // Given two hooks: first vetoes, second would panic.
        let lua = test_lua();
        let veto = lua
            .create_function(|_, _args: (String, Value)| Ok(false))
            .expect("veto callback");
        let boom = lua
            .create_function(|_, _args: (String, Value)| -> Result<bool, mlua::Error> {
                panic!("second hook should not run")
            })
            .expect("panic callback");
        register(&lua, "test::cmd".to_owned(), veto);
        register(&lua, "test::cmd".to_owned(), boom);

        // When dispatching preflight.
        let result = dispatch(&lua, "test::cmd", &serde_json::Value::Null);

        // Then it returns false without running the second hook.
        assert!(!result, "should block on first veto");
    }
}
