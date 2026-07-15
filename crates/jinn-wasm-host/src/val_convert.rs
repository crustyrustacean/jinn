//! JSON ↔ `wasmtime::component::Val` bridge for the dynamic hook calling convention.
//!
//! Sync render hooks (`on-chat-input-badges-render`, `on-keybind-trigger`,
//! `on-submit-intercept`) keep the old `serde_json::Value` trait shape so the
//! TUI call sites don't change. But WASM components speak typed `Val`s. This
//! module converts between the two for the record-shaped ctx params and the
//! option-of-record results those hooks produce.
//!
//! This is a **best-effort** converter covering the WIT types the sync hooks
//! actually use: records, strings, numbers, bools, lists, options, enums, and
//! variants. It is intentionally lossy around exotic types — a malformed plugin
//! degrades (returns null) rather than trapping the render thread.

use serde_json::Value;
use wasmtime::component::Val;

/// Convert a JSON object into a `Val::Record`, matching the WIT record's field
/// names. Non-object JSON is wrapped as a single-field record.
#[must_use]
pub fn json_to_val(json: &Value) -> Val {
    match json {
        Value::Null => Val::Option(None),
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Val::S64(i)
            } else if let Some(u) = n.as_u64() {
                Val::U64(u)
            } else if let Some(f) = n.as_f64() {
                Val::Float64(f)
            } else {
                Val::String(n.to_string())
            }
        }
        Value::String(s) => Val::String(s.clone()),
        Value::Array(arr) => Val::List(arr.iter().map(json_to_val).collect()),
        Value::Object(obj) => Val::Record(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_val(v)))
                .collect(),
        ),
    }
}

/// Convert a `Val` result back into JSON for the trait-shaped return.
///
/// `Option(None)` → JSON null (filtered by `call_hooks` as "no return"). Any
/// unhandled variant also yields null.
#[must_use]
pub fn val_to_json(val: &Val) -> Value {
    match val {
        Val::Bool(b) => Value::Bool(*b),
        Val::S8(i) => Value::Number((*i).into()),
        Val::U8(i) => Value::Number((*i).into()),
        Val::S16(i) => Value::Number((*i).into()),
        Val::U16(i) => Value::Number((*i).into()),
        Val::S32(i) => Value::Number((*i).into()),
        Val::U32(i) => Value::Number((*i).into()),
        Val::S64(i) => Value::Number((*i).into()),
        Val::U64(i) => Value::Number((*i).into()),
        Val::Float32(f) => serde_json::Number::from_f64(f64::from(*f))
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Val::Float64(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Val::Char(c) => Value::String(c.to_string()),
        Val::String(s) => Value::String(s.clone()),
        Val::List(items) => Value::Array(items.iter().map(val_to_json).collect()),
        Val::Map(pairs) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in pairs {
                if let Val::String(key) = k {
                    obj.insert(key.clone(), val_to_json(v));
                }
            }
            Value::Object(obj)
        }
        Val::Record(fields) => {
            let mut obj = serde_json::Map::new();
            for (name, v) in fields {
                obj.insert(name.clone(), val_to_json(v));
            }
            Value::Object(obj)
        }
        Val::Tuple(items) => Value::Array(items.iter().map(val_to_json).collect()),
        Val::Variant(disc, payload) => {
            let mut obj = serde_json::Map::new();
            obj.insert(disc.clone(), Value::Null);
            if let Some(inner) = payload {
                if let Value::Object(mut inner_obj) = val_to_json(inner) {
                    obj.append(&mut inner_obj);
                }
            }
            Value::Object(obj)
        }
        Val::Enum(name) => Value::String(name.clone()),
        Val::Option(inner) => match inner {
            None => Value::Null,
            Some(b) => val_to_json(b),
        },
        Val::Result(res) => match res {
            Ok(Some(b)) => val_to_json(b),
            Ok(None) => Value::Null,
            Err(Some(b)) => val_to_json(b),
            Err(None) => Value::Null,
        },
        Val::Flags(names) => Value::Array(names.iter().map(|n| Value::String(n.clone())).collect()),
        Val::Resource(_) | Val::Future(_) | Val::Stream(_) | Val::ErrorContext(_) => Value::Null,
    }
}
