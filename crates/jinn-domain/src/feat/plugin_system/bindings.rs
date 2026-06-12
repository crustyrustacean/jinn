//! Lua↔JSON conversion utilities.
//!
//! Provides bidirectional conversion between Lua values and `serde_json::Value`.
//! Used by [`PluginRegistry`](crate::registry::PluginRegistry) to translate
//! Rust context structs to Lua tables and Lua return values back to JSON.

use mlua::{Lua, Value};

/// Converts a Lua value to a `serde_json::Value`.
#[expect(
    clippy::only_used_in_recursion,
    reason = "lua parameter needed for Table -> recursive value_to_json calls"
)]
pub(crate) fn value_to_json(lua: &Lua, value: &Value) -> Result<serde_json::Value, mlua::Error> {
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
pub(crate) fn json_to_lua_value(
    lua: &Lua,
    value: &serde_json::Value,
) -> Result<Value, mlua::Error> {
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

    use super::*;

    // --- value_to_json tests ---

    #[rstest::rstest]
    fn value_to_json_converts_sequential_array() {
        // Given a Lua table with sequential integer keys.
        let lua = Lua::new();
        let table = lua.create_table().expect("table");
        table.set(1, "a").expect("set");
        table.set(2, "b").expect("set");
        table.set(3, "c").expect("set");

        // When converting to JSON.
        let json = value_to_json(&lua, &Value::Table(table)).expect("convert");

        // Then it's a JSON array.
        assert!(json.is_array(), "should be a JSON array, got: {json:?}");
        let arr = json.as_array().expect("array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0], serde_json::Value::String("a".to_owned()));
        assert_eq!(arr[1], serde_json::Value::String("b".to_owned()));
        assert_eq!(arr[2], serde_json::Value::String("c".to_owned()));
    }

    #[rstest::rstest]
    fn value_to_json_converts_mixed_keys_to_object() {
        // Given a Lua table with mixed keys.
        let lua = Lua::new();
        lua.load(
            r#"
                local t = { "first", foo = "bar", [5] = "fifth" }
                return t
            "#,
        )
        .eval::<Value>()
        .expect("lua eval");

        // When converting to JSON.
        let json = value_to_json(
            &lua,
            &lua.load("return { 'first', foo = 'bar', [5] = 'fifth' }")
                .eval::<Value>()
                .expect("eval"),
        )
        .expect("convert");

        // Then it's a JSON object (not an array) because keys are not sequential 1..N.
        assert!(
            json.is_object(),
            "mixed-key table should be an object, got: {json:?}"
        );
    }

    #[rstest::rstest]
    fn value_to_json_converts_empty_table_to_empty_object() {
        // Given an empty Lua table.
        let lua = Lua::new();
        let table = lua.create_table().expect("table");

        // When converting to JSON.
        let json = value_to_json(&lua, &Value::Table(table)).expect("convert");

        // Then it's an empty JSON object.
        assert!(
            json.is_object(),
            "empty table should be an object, got: {json:?}"
        );
    }

    // --- json_to_lua_value tests ---

    #[rstest::rstest]
    fn json_to_lua_value_converts_array_to_table_with_correct_indices() {
        // Given a JSON array.
        let lua = Lua::new();
        let json = serde_json::json!([10, 20, 30]);

        // When converting to Lua.
        let value = json_to_lua_value(&lua, &json).expect("convert");

        // Then it's a Lua table with values at indices 1, 2, 3.
        match value {
            Value::Table(t) => {
                assert_eq!(t.get::<i64>(1).unwrap(), 10);
                assert_eq!(t.get::<i64>(2).unwrap(), 20);
                assert_eq!(t.get::<i64>(3).unwrap(), 30);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn json_to_lua_value_converts_object_to_table_with_string_keys() {
        // Given a JSON object.
        let lua = Lua::new();
        let json = serde_json::json!({ "name": "test", "count": 42 });

        // When converting to Lua.
        let value = json_to_lua_value(&lua, &json).expect("convert");

        // Then it's a Lua table with string keys.
        match value {
            Value::Table(t) => {
                assert_eq!(t.get::<String>("name").unwrap(), "test");
                assert_eq!(t.get::<i64>("count").unwrap(), 42);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn json_to_lua_value_converts_null_to_nil() {
        // Given a JSON null.
        let lua = Lua::new();
        let json = serde_json::Value::Null;

        // When converting to Lua.
        let value = json_to_lua_value(&lua, &json).expect("convert");

        // Then it's Nil.
        assert!(matches!(value, Value::Nil));
    }

    #[rstest::rstest]
    fn json_to_lua_value_converts_nested_structure() {
        // Given a nested JSON structure.
        let lua = Lua::new();
        let json = serde_json::json!({
            "items": [1, 2, 3],
            "nested": { "key": "value" }
        });

        // When converting to Lua.
        let value = json_to_lua_value(&lua, &json).expect("convert");

        // Then the nested structure is preserved.
        match value {
            Value::Table(t) => {
                let items: Value = t.get("items").unwrap();
                match items {
                    Value::Table(arr) => {
                        assert_eq!(arr.raw_len(), 3);
                        assert_eq!(arr.get::<i64>(1).unwrap(), 1);
                        assert_eq!(arr.get::<i64>(2).unwrap(), 2);
                        assert_eq!(arr.get::<i64>(3).unwrap(), 3);
                    }
                    other => panic!("expected items to be Table, got {other:?}"),
                }
                let nested: Value = t.get("nested").unwrap();
                match nested {
                    Value::Table(obj) => {
                        assert_eq!(obj.get::<String>("key").unwrap(), "value");
                    }
                    other => panic!("expected nested to be Table, got {other:?}"),
                }
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }
}
