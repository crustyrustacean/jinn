//! Lua bindings for plugin communication.
//!
//! Installs `ns` and `ps` global tables into the Lua VM:
//!
//! - `ns.emit(name, payload)` — emit a dynamic command
//! - `ps.sub(name, callback)` — subscribe to an event name
//! - `ps.pub(name, payload)` — publish to event subscribers
//! - `ps.unsub(name)` — unsubscribe from an event name

use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Function, Lua, Value};
use parking_lot::Mutex;

use crate::host::CommandSender;

/// Internal subscription map stored in Lua app_data.
///
/// Wrapped in a newtype to distinguish from [`PreflightMap`] in mlua's
/// type-indexed `app_data` storage.
pub(crate) type SubscriptionMap = Arc<Mutex<HashMap<String, Vec<Function>>>>;

/// Wrapper to make [`SubscriptionMap`] a distinct type for mlua `app_data`.
#[derive(Clone)]
pub(crate) struct Subscriptions {
    /// Inner subscription map.
    inner: SubscriptionMap,
}

impl Subscriptions {
    /// Creates a new `Subscriptions` wrapper.
    pub(crate) fn new(inner: SubscriptionMap) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner subscription map.
    pub(crate) fn get(&self) -> &SubscriptionMap {
        &self.inner
    }
}

/// Installs the `ns` and `ps` global tables into the Lua VM.
///
/// The `ns.emit` binding calls `sender` with a constructed
/// `Command::Dynamic`. The `ps` bindings manage an internal
/// subscription map for in-VM pub/sub.
pub fn install(lua: &Lua, sender: &CommandSender) -> Result<(), mlua::Error> {
    #[expect(clippy::arc_with_non_send_sync, reason = "mlua::Function is not Send but only used from the single Lua thread")]
    let subs: SubscriptionMap = Arc::new(Mutex::new(HashMap::new()));
    lua.set_app_data(Subscriptions::new(subs.clone()));

    // ns table — namespace for nullslop commands.
    let ns = lua.create_table()?;
    {
        let sender = sender.clone();
        let ns_emit = lua.create_function(move |lua, (name, payload): (String, Value)| {
            let json_payload = value_to_json(lua, &payload)?;
            let cmd = nullslop_domain::Command::Dynamic(
                nullslop_domain::DynamicCommand {
                    name,
                    payload: json_payload,
                },
            );
            sender.send(cmd);
            Ok(())
        })?;
        ns.set("emit", ns_emit)?;
    }

    // ps table — pub/sub within the Lua VM.
    let ps = lua.create_table()?;
    {
        let subs_ref = subs.clone();
        let ps_sub = lua.create_function(move |_lua, (name, callback): (String, Function)| {
            let mut map = subs_ref.lock();
            map.entry(name).or_default().push(callback);
            Ok(())
        })?;
        ps.set("sub", ps_sub)?;
    }
    {
        let subs_ref = subs.clone();
        let ps_pub = lua.create_function(move |_lua, (name, payload): (String, Value)| {
            let callbacks: Vec<Function> = {
                let map = subs_ref.lock();
                map.get(&name).cloned().unwrap_or_default()
            };
            for callback in callbacks {
                callback.call::<()>(payload.clone())?;
            }
            Ok(())
        })?;
        ps.set("pub", ps_pub)?;
    }
    {
        let subs_ref = subs;
        let ps_unsub = lua.create_function(move |_lua, name: String| {
            let mut map = subs_ref.lock();
            map.remove(&name);
            Ok(())
        })?;
        ps.set("unsub", ps_unsub)?;
    }

    lua.globals().set("ns", ns)?;
    lua.globals().set("ps", ps)?;

    Ok(())
}

/// Converts a Lua value to a `serde_json::Value`.
#[expect(clippy::only_used_in_recursion, reason = "lua parameter needed for Table -> recursive value_to_json calls")]
fn value_to_json(lua: &Lua, value: &Value) -> Result<serde_json::Value, mlua::Error> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::json!(*i)),
        Value::Number(n) => Ok(serde_json::json!(*n)),
        Value::String(s) => Ok(serde_json::Value::String(s.to_string_lossy())),
        Value::Table(t) => {
            // Collect all key-value pairs.
            let pairs: Vec<(Value, Value)> = t.pairs().collect::<Result<Vec<_>, _>>()?;
            if pairs.is_empty() {
                return Ok(serde_json::Value::Object(serde_json::Map::new()));
            }

            // If all keys are integers 1..N, treat as array.
            let mut int_keys = Vec::new();
            let mut all_int_keys = true;
            for (k, _) in &pairs {
                if let Value::Integer(i) = k {
                    int_keys.push(*i);
                } else {
                    all_int_keys = false;
                    break;
                }
            }

            if all_int_keys && !int_keys.is_empty() {
                int_keys.sort_unstable();
                let sequential = int_keys
                    .iter()
                    .enumerate()
                    .all(|(i, k)| *k == i64::try_from(i + 1).unwrap_or(0));
                if sequential {
                    let count = int_keys.len();
                    let mut arr = Vec::with_capacity(count);
                    for i in 1..=count {
                        let v: Value = t.get(i)?;
                        arr.push(value_to_json(lua, &v)?);
                    }
                    return Ok(serde_json::Value::Array(arr));
                }
            }

            // Otherwise treat as object.
            let mut map = serde_json::Map::new();
            for (k, v) in &pairs {
                let key_str = match k {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    other => format!("{other:?}"),
                };
                map.insert(key_str, value_to_json(lua, v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        other => Ok(serde_json::json!(format!("{other:?}"))),
    }
}

/// Converts a `serde_json::Value` to a Lua value.
pub(crate) fn json_to_lua_value(lua: &Lua, value: &serde_json::Value) -> Result<Value, mlua::Error> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else {
                Ok(Value::Number(
                    n.as_f64().unwrap_or(0.0),
                ))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_to_lua_value(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
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
    use nullslop_domain::Command;

    use super::*;

    fn test_setup() -> (Lua, kanal::Receiver<Command>) {
        let lua = Lua::new();
        let (tx, rx) = kanal::unbounded();
        let sender = CommandSender::new(move |cmd: Command| {
            let _ = tx.send(cmd);
        });
        install(&lua, &sender).expect("install bindings");
        (lua, rx)
    }

    #[rstest::rstest]
    fn ns_emit_sends_dynamic_command() {
        // Given a Lua VM with bindings installed.
        let (lua, rx) = test_setup();

        // When calling ns.emit("test::cmd", { foo = "bar" }).
        lua.load(r#"ns.emit("test::cmd", { foo = "bar" })"#)
            .exec()
            .expect("lua exec");

        // Then a Dynamic command is received.
        let cmd = rx.recv().expect("receive command");
        match cmd {
            Command::Dynamic(dc) => {
                assert_eq!(dc.name, "test::cmd");
                assert_eq!(dc.payload["foo"], "bar");
            }
            other => panic!("expected Dynamic, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn ns_emit_with_nil_payload_sends_null() {
        // Given a Lua VM with bindings.
        let (lua, rx) = test_setup();

        // When calling ns.emit("test::cmd", nil).
        lua.load(r#"ns.emit("test::cmd", nil)"#)
            .exec()
            .expect("lua exec");

        // Then the payload is Value::Null.
        let cmd = rx.recv().expect("receive command");
        match cmd {
            Command::Dynamic(dc) => {
                assert_eq!(dc.name, "test::cmd");
                assert!(dc.payload.is_null(), "nil payload should be Value::Null");
            }
            other => panic!("expected Dynamic, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn ps_pub_dispatches_to_subscriber() {
        // Given a Lua VM with bindings.
        let (lua, _) = test_setup();

        // When subscribing and publishing.
        lua.load(
            r#"
                flag = false
                ps.sub("x", function(payload)
                    flag = true
                end)
                ps.pub("x", {})
            "#,
        )
        .exec()
        .expect("lua exec");

        // Then the callback was invoked.
        let flag: bool = lua.globals().get("flag").expect("get flag");
        assert!(flag, "callback should have been invoked");
    }

    #[rstest::rstest]
    fn ps_sub_callback_receives_correct_payload() {
        // Given a Lua VM with bindings.
        let (lua, _) = test_setup();

        // When subscribing with a callback that captures the payload.
        lua.load(
            r#"
                received = nil
                ps.sub("y", function(payload)
                    received = payload.msg
                end)
                ps.pub("y", { msg = "hello" })
            "#,
        )
        .exec()
        .expect("lua exec");

        // Then the callback received the correct payload.
        let received: String = lua.globals().get("received").expect("get received");
        assert_eq!(received, "hello");
    }

    #[rstest::rstest]
    fn ps_unsub_stops_delivery() {
        // Given a Lua VM with bindings.
        let (lua, _) = test_setup();

        // When subscribing, unsubscribing, then publishing.
        lua.load(
            r#"
                flag = false
                ps.sub("z", function(payload)
                    flag = true
                end)
                ps.unsub("z")
                ps.pub("z", {})
            "#,
        )
        .exec()
        .expect("lua exec");

        // Then the callback was NOT invoked.
        let flag: bool = lua.globals().get("flag").expect("get flag");
        assert!(!flag, "callback should not have been invoked after unsub");
    }
}
