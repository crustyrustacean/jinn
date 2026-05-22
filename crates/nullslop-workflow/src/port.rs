//! Port types — the data model for workflow node I/O.
//!
//! Every node declares its input and output ports as [`PortDef`]s with a
//! [`PortType`]. Data flows between ports as [`PortValue`]s collected in
//! a [`PortValues`] map.

use std::collections::HashMap;

use derive_more::Display;
use wherror::Error;

/// The types a port can carry. Extensible enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortType {
    /// A plain string value.
    String,
    /// A JSON value (object, array, number, bool, string, null).
    Json,
}

impl PortType {
    /// Returns the [`PortType`] for a given [`PortValue`].
    #[must_use]
    pub fn from_value(value: &PortValue) -> Self {
        match value {
            PortValue::String(_) => Self::String,
            PortValue::Json(_) => Self::Json,
        }
    }
}

/// A named, typed port definition on a node.
///
/// Ports are the connection points — think of them as pins on a
/// blueprint node. Each port has a name (for routing) and a type
/// (for validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDef {
    /// The port name (e.g., `"prompt"`, `"response"`, `"data"`).
    pub name: &'static str,
    /// The type this port accepts or produces.
    pub value_type: PortType,
}

impl PortDef {
    /// Creates a new port definition.
    #[must_use]
    pub fn new(name: &'static str, value_type: PortType) -> Self {
        Self { name, value_type }
    }

    /// Convenience: creates a `String`-typed port definition.
    #[must_use]
    pub fn string(name: &'static str) -> Self {
        Self::new(name, PortType::String)
    }

    /// Convenience: creates a `Json`-typed port definition.
    #[must_use]
    pub fn json(name: &'static str) -> Self {
        Self::new(name, PortType::Json)
    }
}

/// A single value flowing through a port.
#[derive(Debug, Clone, PartialEq)]
pub enum PortValue {
    /// A string value.
    String(String),
    /// A JSON value.
    Json(serde_json::Value),
}

impl PortValue {
    /// Returns `true` if this is a string value.
    #[must_use]
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    /// Returns `true` if this is a JSON value.
    #[must_use]
    pub fn is_json(&self) -> bool {
        matches!(self, Self::Json(_))
    }

    /// Returns the [`PortType`] of this value.
    #[must_use]
    pub fn port_type(&self) -> PortType {
        PortType::from_value(self)
    }

    /// Extracts the inner string, returning an error if the type doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::TypeMismatch`] if this is not a `String` value.
    pub fn into_string(self) -> Result<String, PortError> {
        match self {
            Self::String(s) => Ok(s),
            actual => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::String,
                actual: PortType::from_value(&actual),
            }),
        }
    }

    /// Extracts the inner JSON value, returning an error if the type doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::TypeMismatch`] if this is not a `Json` value.
    pub fn into_json(self) -> Result<serde_json::Value, PortError> {
        match self {
            Self::Json(v) => Ok(v),
            actual => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::Json,
                actual: PortType::from_value(&actual),
            }),
        }
    }
}

impl From<String> for PortValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for PortValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_owned())
    }
}

impl From<serde_json::Value> for PortValue {
    fn from(v: serde_json::Value) -> Self {
        Self::Json(v)
    }
}

/// A map from port name to its value.
///
/// The engine guarantees all declared input ports are present
/// before calling [`execute`](crate::node::WorkflowNode::execute).
#[derive(Debug, Clone, Default)]
pub struct PortValues(HashMap<String, PortValue>);

impl PortValues {
    /// Creates an empty `PortValues`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value, returning the previous value for that port (if any).
    pub fn insert(&mut self, name: impl Into<String>, value: PortValue) -> Option<PortValue> {
        self.0.insert(name.into(), value)
    }

    /// Returns `true` if the map contains a value for the given port name.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Returns the number of port values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are no port values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Gets a reference to a value by port name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PortValue> {
        self.0.get(name)
    }

    /// Gets a reference to a string value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-string value.
    pub fn get_string(&self, name: &str) -> Result<&str, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::String(s)) => Ok(s),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::String,
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Gets a reference to a JSON value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-JSON value.
    pub fn get_json(&self, name: &str) -> Result<&serde_json::Value, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Json(v)) => Ok(v),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Json,
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Removes and returns a value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    pub fn take(&mut self, name: &str) -> Result<PortValue, PortError> {
        self.0
            .remove(name)
            .ok_or_else(|| PortError::Missing {
                name: name.to_owned(),
            })
    }

    /// Removes and returns a string value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-string value.
    pub fn take_string(&mut self, name: &str) -> Result<String, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::String(s)) => Ok(s),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::String,
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Removes and returns a JSON value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-JSON value.
    pub fn take_json(&mut self, name: &str) -> Result<serde_json::Value, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Json(v)) => Ok(v),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Json,
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Returns an iterator over port name-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PortValue)> {
        self.0.iter()
    }
}

impl From<HashMap<String, PortValue>> for PortValues {
    fn from(map: HashMap<String, PortValue>) -> Self {
        Self(map)
    }
}

/// Errors arising from port access.
#[derive(Debug, Error, Display)]
pub enum PortError {
    /// The requested port was not found.
    #[display("port '{name}' not found")]
    Missing {
        /// The port name that was requested.
        name: String,
    },
    /// The port had the wrong type.
    #[display("port '{name}' expected {expected:?} but got {actual:?}")]
    TypeMismatch {
        /// The port name.
        name: String,
        /// The expected type.
        expected: PortType,
        /// The actual type.
        actual: PortType,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_def_string_creates_string_typed_port() {
        // Given a name for a string port.
        let def = PortDef::string("prompt");

        // When creating via convenience method.
        // Then it has String type.
        assert_eq!(def.name, "prompt");
        assert_eq!(def.value_type, PortType::String);
    }

    #[test]
    fn port_def_json_creates_json_typed_port() {
        // Given a name for a JSON port.
        let def = PortDef::json("data");

        // When creating via convenience method.
        // Then it has Json type.
        assert_eq!(def.name, "data");
        assert_eq!(def.value_type, PortType::Json);
    }

    #[test]
    fn take_string_returns_value_when_present() {
        // Given a PortValues with a string port.
        let mut values = PortValues::new();
        values.insert("prompt", PortValue::String("hello".to_owned()));

        // When taking the string value.
        let result = values.take_string("prompt");

        // Then it returns the value.
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn take_string_returns_missing_when_absent() {
        // Given an empty PortValues.
        let mut values = PortValues::new();

        // When taking a non-existent port.
        let result = values.take_string("missing");

        // Then it returns Missing error.
        assert!(matches!(result, Err(PortError::Missing { .. })));
    }

    #[test]
    fn take_string_returns_type_mismatch_for_json_value() {
        // Given a PortValues with a JSON port.
        let mut values = PortValues::new();
        values.insert("data", PortValue::Json(serde_json::json!({"key": 42})));

        // When taking as a string.
        let result = values.take_string("data");

        // Then it returns TypeMismatch error.
        assert!(matches!(
            result,
            Err(PortError::TypeMismatch {
                expected: PortType::String,
                actual: PortType::Json,
                ..
            })
        ));
    }

    #[test]
    fn get_string_returns_reference_to_value() {
        // Given a PortValues with a string port.
        let mut values = PortValues::new();
        values.insert("prompt", PortValue::String("hello".to_owned()));

        // When getting the string reference.
        let result = values.get_string("prompt");

        // Then it returns a reference to the value.
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn get_json_returns_reference_to_value() {
        // Given a PortValues with a JSON port.
        let mut values = PortValues::new();
        let json = serde_json::json!({"key": "value"});
        values.insert("data", PortValue::Json(json.clone()));

        // When getting the JSON reference.
        let result = values.get_json("data");

        // Then it returns a reference to the value.
        assert_eq!(result.unwrap(), &json);
    }

    #[test]
    fn get_json_returns_missing_when_absent() {
        // Given an empty PortValues.
        let values = PortValues::new();

        // When getting a non-existent port.
        let result = values.get_json("missing");

        // Then it returns Missing error.
        assert!(matches!(result, Err(PortError::Missing { .. })));
    }

    #[test]
    fn insert_and_take_roundtrip() {
        // Given a PortValues with multiple ports.
        let mut values = PortValues::new();
        values.insert("text", PortValue::String("hello".to_owned()));
        values.insert("data", PortValue::Json(serde_json::json!([1, 2, 3])));

        // When taking all values back.
        let text = values.take_string("text").unwrap();
        let data = values.take_json("data").unwrap();

        // Then both values are recovered correctly.
        assert_eq!(text, "hello");
        assert_eq!(data, serde_json::json!([1, 2, 3]));
        // And the map is now empty for those keys.
        assert!(!values.contains("text"));
        assert!(!values.contains("data"));
    }

    #[test]
    fn contains_returns_true_for_present_port() {
        // Given a PortValues with a port.
        let mut values = PortValues::new();
        values.insert("name", PortValue::String("test".to_owned()));

        // When checking if the port exists.
        // Then it returns true.
        assert!(values.contains("name"));
        assert!(!values.contains("other"));
    }

    #[test]
    fn len_returns_number_of_ports() {
        // Given a PortValues with two ports.
        let mut values = PortValues::new();
        values.insert("a", PortValue::String("1".to_owned()));
        values.insert("b", PortValue::String("2".to_owned()));

        // When checking length.
        // Then it returns 2.
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn from_str_creates_string_port_value() {
        // Given a &str.
        let value: PortValue = "hello".into();

        // When converting to PortValue.
        // Then it is a String variant.
        assert_eq!(value, PortValue::String("hello".to_owned()));
    }

    #[test]
    fn port_value_port_type_returns_correct_type() {
        // Given string and JSON values.
        let string_val = PortValue::String("test".to_owned());
        let json_val = PortValue::Json(serde_json::json!(null));

        // When getting their port types.
        // Then they match their variants.
        assert_eq!(string_val.port_type(), PortType::String);
        assert_eq!(json_val.port_type(), PortType::Json);
    }

    #[test]
    fn iter_yields_all_port_pairs() {
        // Given a PortValues with two ports.
        let mut values = PortValues::new();
        values.insert("a", PortValue::String("1".to_owned()));
        values.insert("b", PortValue::Json(serde_json::json!(2)));

        // When iterating.
        let pairs: Vec<_> = values.iter().collect();

        // Then both pairs are present.
        assert_eq!(pairs.len(), 2);
    }
}
