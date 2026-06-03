//! Ctx builder — constructs the Lua ctx table with data and capability methods.
//!
//! [`CtxBuilder`] takes a serializable data struct and registers async
//! capability methods via [`with_method`]. The resulting ctx table is passed
//! to the Lua script's `run(ctx)` function.

use error_stack::Report;
use mlua::{Function, Lua, Table, Value};
use serde::Serialize;

use crate::protocol::HostRequest;
use crate::registry::LuaError;

/// Builder for constructing the Lua ctx table.
///
/// Created with serializable data, then capability methods are added via
/// [`with_method`]. The builder has zero knowledge of what capabilities
/// exist — it's purely mechanical.
pub struct CtxBuilder<'a> {
    lua: &'a Lua,
    table: Table,
}

impl<'a> CtxBuilder<'a> {
    /// Creates a new builder with the given data serialized into the ctx table.
    ///
    /// The data struct's fields become Lua table keys. Use
    /// `#[serde(rename_all = "snake_case")]` for consistent naming.
    pub fn new<S: Serialize>(lua: &'a Lua, data: &S) -> Result<Self, Report<LuaError>> {
        let table = lua
            .create_table()
            .map_err(|e| LuaError::script(format!("create ctx table: {e}")))?;

        // Serialize data fields into the table.
        let json = serde_json::to_value(data)
            .map_err(|e| LuaError::script(format!("serialize ctx data: {e}")))?;

        if let serde_json::Value::Object(map) = json {
            for (key, value) in map {
                let lua_value = json_to_lua_value(lua, &value)?;
                table
                    .set(key, lua_value)
                    .map_err(|e| LuaError::script(format!("set ctx field: {e}")))?;
            }
        }

        Ok(Self { lua, table })
    }

    /// Creates a new builder with an empty ctx table (no data).
    pub fn empty(lua: &'a Lua) -> Result<Self, Report<LuaError>> {
        let table = lua
            .create_table()
            .map_err(|e| LuaError::script(format!("create ctx table: {e}")))?;
        Ok(Self { lua, table })
    }

    /// Registers a pre-built function on the ctx table.
    ///
    /// The caller is responsible for creating the function (sync or async).
    /// This gives full control over the function signature and lifetime.
    pub fn with_function(mut self, name: &str, func: Function) -> Result<Self, Report<LuaError>> {
        self.table
            .set(name, func)
            .map_err(|e| LuaError::script(format!("set method '{name}': {e}")))?;
        Ok(self)
    }

    /// Sets a raw value on the ctx table.
    pub fn with_value<V: Into<Value>>(
        mut self,
        name: &str,
        value: V,
    ) -> Result<Self, Report<LuaError>> {
        self.table
            .set(name, value.into())
            .map_err(|e| LuaError::script(format!("set value '{name}': {e}")))?;
        Ok(self)
    }

    /// Returns a reference to the Lua instance.
    pub fn lua(&self) -> &'a Lua {
        self.lua
    }

    /// Returns the built ctx table.
    pub fn build(self) -> Table {
        self.table
    }
}

/// Converts a `serde_json::Value` to a `mlua::Value`.
pub fn json_to_lua_value(lua: &Lua, value: &serde_json::Value) -> Result<Value, Report<LuaError>> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else {
                Ok(Value::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => {
            let lua_str = lua
                .create_string(s)
                .map_err(|e| LuaError::script(format!("create string: {e}")))?;
            Ok(Value::String(lua_str))
        }
        serde_json::Value::Array(arr) => {
            let table = lua
                .create_table()
                .map_err(|e| LuaError::script(format!("create array table: {e}")))?;
            for (i, v) in arr.iter().enumerate() {
                let lua_v = json_to_lua_value(lua, v)?;
                table
                    .set(i64::try_from(i + 1).unwrap_or(0), lua_v)
                    .map_err(|e| LuaError::script(format!("set array element: {e}")))?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua
                .create_table()
                .map_err(|e| LuaError::script(format!("create object table: {e}")))?;
            for (k, v) in map {
                let lua_v = json_to_lua_value(lua, v)?;
                table
                    .set(k.as_str(), lua_v)
                    .map_err(|e| LuaError::script(format!("set object field: {e}")))?;
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

    use super::*;

    #[derive(serde::Serialize)]
    struct TestData {
        message: String,
        count: i64,
    }

    #[test]
    fn ctx_builder_serializes_data() {
        let lua = Lua::new();
        let data = TestData {
            message: "hello".to_owned(),
            count: 42,
        };

        let builder = CtxBuilder::new(&lua, &data).expect("builder");
        let ctx = builder.build();

        let msg: String = ctx.get("message").expect("get message");
        assert_eq!(msg, "hello");

        let count: i64 = ctx.get("count").expect("get count");
        assert_eq!(count, 42);
    }

    #[test]
    fn ctx_builder_with_function_adds_callable() {
        let lua = Lua::new();
        let data = TestData {
            message: "hello".to_owned(),
            count: 42,
        };

        let double_fn = lua
            .create_function(|_lua, n: i64| Ok(n * 2))
            .expect("create fn");

        let builder = CtxBuilder::new(&lua, &data)
            .expect("builder")
            .with_function("double", double_fn)
            .expect("with_function");

        let ctx = builder.build();

        let f: Function = ctx.get("double").expect("get double");
        let result: i64 = f.call(21).expect("call double");
        assert_eq!(result, 42);
    }

    #[test]
    fn ctx_builder_empty_data() {
        #[derive(serde::Serialize)]
        struct EmptyData;

        let lua = Lua::new();
        let data = EmptyData {};
        let builder = CtxBuilder::new(&lua, &data).expect("builder");
        let ctx = builder.build();

        let pairs: Vec<(Value, Value)> = ctx.pairs().collect::<Result<Vec<_>, _>>().expect("pairs");
        assert!(pairs.is_empty());
    }

    #[test]
    fn ctx_builder_nested_data() {
        #[derive(serde::Serialize)]
        struct NestedData {
            items: Vec<String>,
        }

        let lua = Lua::new();
        let data = NestedData {
            items: vec!["a".to_owned(), "b".to_owned()],
        };

        let builder = CtxBuilder::new(&lua, &data).expect("builder");
        let ctx = builder.build();

        let items: mlua::Table = ctx.get("items").expect("get items");
        let first: String = items.get(1).expect("get first");
        assert_eq!(first, "a");
        let second: String = items.get(2).expect("get second");
        assert_eq!(second, "b");
    }

    #[test]
    fn ctx_builder_with_async_function() {
        let lua = Lua::new();

        let async_fn = lua
            .create_async_function(|_lua, n: i64| async move {
                tokio::task::yield_now().await;
                Ok(n + 10)
            })
            .expect("create async fn");

        let builder = CtxBuilder::empty(&lua)
            .expect("builder")
            .with_function("add_ten", async_fn)
            .expect("with_function");

        let ctx = builder.build();
        let f: Function = ctx.get("add_ten").expect("get add_ten");

        // call_async on the current thread
        let rt = tokio::runtime::Runtime::new().expect("rt");
        let local = tokio::task::LocalSet::new();
        let result: i64 = local.block_on(&rt, f.call_async(5)).expect("call");
        assert_eq!(result, 15);
    }
}
