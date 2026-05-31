//! Port types - the data model for workflow node I/O.
//!
//! Every node declares its input and output ports as [`PortDef`]s with a
//! [`PortType`]. Data flows between ports as [`PortValue`]s collected in
//! a [`PortValues`] map.
//!
//! # Type model
//!
//! The type system has two levels:
//!
//! - **[`ScalarType`]** - the fundamental data kinds: `Text`, `Number`, `Boolean`, `Json`.
//! - **[`PortType`]** - containers wrapping scalar types: `Single`, `Vector`, `Map`.
//!
//! Containers are homogeneous: `Vector(Number)` holds only numbers,
//! `Map(Text)` holds only text values. Mixed-type data uses the `Json`
//! escape hatch.

use std::collections::HashMap;

use derive_more::Display;
use wherror::Error;

/// Semantic scalar types - the fundamental data kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarType {
    /// A text string.
    Text,
    /// A numeric value (64-bit float).
    Number,
    /// A boolean value.
    Boolean,
    /// A JSON value (object, array, number, bool, string, null) - escape hatch.
    Json,
}

impl ScalarType {
    /// Returns the [`ScalarType`] for a given [`ScalarValue`].
    #[must_use]
    pub fn from_value(value: &ScalarValue) -> Self {
        match value {
            ScalarValue::Text(_) => Self::Text,
            ScalarValue::Number(_) => Self::Number,
            ScalarValue::Boolean(_) => Self::Boolean,
            ScalarValue::Json(_) => Self::Json,
        }
    }

    /// Returns a short display label for this scalar type.
    ///
    /// Used for layout computation and rendering.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            ScalarType::Text => "Text",
            ScalarType::Number => "Num",
            ScalarType::Boolean => "Bool",
            ScalarType::Json => "Json",
        }
    }
}

/// Container types wrapping scalar types.
///
/// Each variant parameterizes the inner [`ScalarType`]. Containers are
/// homogeneous - all elements share the same scalar type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortType {
    /// A single scalar value.
    Single(ScalarType),
    /// An ordered list of homogeneous scalar values.
    Vector(ScalarType),
    /// A string-keyed map of homogeneous scalar values.
    Map(ScalarType),
}

impl PortType {
    /// Returns the [`PortType`] for a given [`PortValue`].
    ///
    /// Empty containers default to `ScalarType::Json` for the inner type.
    #[must_use]
    pub fn from_value(value: &PortValue) -> Self {
        match value {
            PortValue::Single(sv) => Self::Single(ScalarType::from_value(sv)),
            PortValue::Vector(items) => {
                let inner = items
                    .first()
                    .map_or(ScalarType::Json, ScalarType::from_value);
                Self::Vector(inner)
            }
            PortValue::Map(entries) => {
                let inner = entries
                    .values()
                    .next()
                    .map_or(ScalarType::Json, ScalarType::from_value);
                Self::Map(inner)
            }
        }
    }

    /// Returns a short display label for this port type.
    ///
    /// Used for layout computation and rendering.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            PortType::Single(scalar) => scalar.label().to_owned(),
            PortType::Vector(scalar) => format!("Vec<{}>", scalar.label()),
            PortType::Map(scalar) => format!("Map<{}>", scalar.label()),
        }
    }
}

/// A named, typed port definition on a node.
///
/// Ports are the connection points - think of them as pins on a
/// blueprint node. Each port has a name (for routing), a type
/// (for validation), and a `required` flag.
///
/// Optional ports (`required: false`) are not checked for incoming edges
/// during graph validation and do not contribute to the pending-input count
/// in the execution engine. Use them for ports that have sensible defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDef {
    /// The port name (e.g., `"prompt"`, `"response"`, `"data"`).
    pub name: String,
    /// The type this port accepts or produces.
    pub value_type: PortType,
    /// Whether this port must be connected for the graph to be valid.
    ///
    /// Defaults to `true`. Set to `false` via [`PortDef::optional`].
    pub required: bool,
}

impl PortDef {
    /// Creates a new port definition.
    #[must_use]
    pub fn new<N>(name: N, value_type: PortType) -> Self
    where
        N: Into<String>,
    {
        Self {
            name: name.into(),
            value_type,
            required: true,
        }
    }

    /// Marks this port as optional (not required to be connected).
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Convenience: creates a `Single(Text)` port definition.
    #[must_use]
    pub fn text<N>(name: N) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Single(ScalarType::Text))
    }

    /// Convenience: creates a `Single(Number)` port definition.
    #[must_use]
    pub fn number<N>(name: N) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Single(ScalarType::Number))
    }

    /// Convenience: creates a `Single(Boolean)` port definition.
    #[must_use]
    pub fn boolean<N>(name: N) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Single(ScalarType::Boolean))
    }

    /// Convenience: creates a `Single(Json)` port definition.
    #[must_use]
    pub fn json<N>(name: N) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Single(ScalarType::Json))
    }

    /// Convenience: creates a `Vector(element_type)` port definition.
    #[must_use]
    pub fn vec_of<N>(name: N, element_type: ScalarType) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Vector(element_type))
    }

    /// Convenience: creates a `Map(value_type)` port definition.
    #[must_use]
    pub fn map_of<N>(name: N, value_type: ScalarType) -> Self
    where
        N: Into<String>,
    {
        Self::new(name, PortType::Map(value_type))
    }
}

/// Runtime scalar values - native Rust payloads.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// A text string.
    Text(String),
    /// A numeric value (64-bit float).
    Number(f64),
    /// A boolean value.
    Boolean(bool),
    /// A JSON value - escape hatch for structured data.
    Json(serde_json::Value),
}

impl ScalarValue {
    /// Returns the [`ScalarType`] of this value.
    #[must_use]
    pub fn scalar_type(&self) -> ScalarType {
        ScalarType::from_value(self)
    }

    /// Extracts the inner text, returning an error if the type doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::TypeMismatch`] if this is not a `Text` value.
    pub fn into_text(self) -> Result<String, PortError> {
        match self {
            Self::Text(s) => Ok(s),
            other => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::Single(ScalarType::Text),
                actual: PortType::Single(ScalarType::from_value(&other)),
            }),
        }
    }

    /// Extracts the inner number, returning an error if the type doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::TypeMismatch`] if this is not a `Number` value.
    pub fn into_number(self) -> Result<f64, PortError> {
        match self {
            Self::Number(n) => Ok(n),
            other => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::Single(ScalarType::Number),
                actual: PortType::Single(ScalarType::from_value(&other)),
            }),
        }
    }

    /// Extracts the inner boolean, returning an error if the type doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::TypeMismatch`] if this is not a `Boolean` value.
    pub fn into_boolean(self) -> Result<bool, PortError> {
        match self {
            Self::Boolean(b) => Ok(b),
            other => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::Single(ScalarType::Boolean),
                actual: PortType::Single(ScalarType::from_value(&other)),
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
            other => Err(PortError::TypeMismatch {
                name: String::new(),
                expected: PortType::Single(ScalarType::Json),
                actual: PortType::Single(ScalarType::from_value(&other)),
            }),
        }
    }
}

/// Runtime port values - containers of scalar values.
///
/// Containers are homogeneous: all elements in a `Vector` share the
/// same [`ScalarType`], and all values in a `Map` share the same
/// [`ScalarType`]. Use the fallible [`PortValue::vector()`] and
/// [`PortValue::map()`] constructors to validate homogeneity.
#[derive(Debug, Clone, PartialEq)]
pub enum PortValue {
    /// A single scalar value.
    Single(ScalarValue),
    /// An ordered list of homogeneous scalar values.
    Vector(Vec<ScalarValue>),
    /// A string-keyed map of homogeneous scalar values.
    Map(HashMap<String, ScalarValue>),
}

impl PortValue {
    /// Creates a `Single` port value wrapping a scalar.
    #[must_use]
    pub fn single(value: ScalarValue) -> Self {
        Self::Single(value)
    }

    /// Creates a `Vector` port value, validating all elements share the same scalar type.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::HeterogeneousContainer`] if elements have mixed scalar types.
    pub fn vector(items: Vec<ScalarValue>) -> Result<Self, PortError> {
        if items.is_empty() {
            return Ok(Self::Vector(items));
        }
        #[expect(clippy::indexing_slicing, reason = "checked empty above")]
        let first_type = ScalarType::from_value(&items[0]);
        for item in &items {
            if ScalarType::from_value(item) != first_type {
                return Err(PortError::HeterogeneousContainer);
            }
        }
        Ok(Self::Vector(items))
    }

    /// Creates a `Map` port value, validating all values share the same scalar type.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::HeterogeneousContainer`] if values have mixed scalar types.
    pub fn map(entries: HashMap<String, ScalarValue>) -> Result<Self, PortError> {
        if entries.is_empty() {
            return Ok(Self::Map(entries));
        }
        let first_type = entries.values().next().map(ScalarType::from_value);
        for value in entries.values() {
            if Some(ScalarType::from_value(value)) != first_type {
                return Err(PortError::HeterogeneousContainer);
            }
        }
        Ok(Self::Map(entries))
    }

    /// Returns the [`PortType`] of this value.
    #[must_use]
    pub fn port_type(&self) -> PortType {
        PortType::from_value(self)
    }
}

impl From<String> for PortValue {
    fn from(s: String) -> Self {
        Self::Single(ScalarValue::Text(s))
    }
}

impl From<&str> for PortValue {
    fn from(s: &str) -> Self {
        Self::Single(ScalarValue::Text(s.to_owned()))
    }
}

impl From<f64> for PortValue {
    fn from(v: f64) -> Self {
        Self::Single(ScalarValue::Number(v))
    }
}

impl From<bool> for PortValue {
    fn from(v: bool) -> Self {
        Self::Single(ScalarValue::Boolean(v))
    }
}

impl From<serde_json::Value> for PortValue {
    fn from(v: serde_json::Value) -> Self {
        Self::Single(ScalarValue::Json(v))
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
    pub fn insert(&mut self, name: String, value: PortValue) -> Option<PortValue> {
        self.0.insert(name, value)
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

    /// Gets a reference to a text value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-text value.
    pub fn get_text(&self, name: &str) -> Result<&str, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Text(s))) => Ok(s),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Text),
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Gets a reference to a number value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-number value.
    pub fn get_number(&self, name: &str) -> Result<f64, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Number(n))) => Ok(*n),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Number),
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Gets a reference to a boolean value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-boolean value.
    pub fn get_boolean(&self, name: &str) -> Result<bool, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Boolean(b))) => Ok(*b),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Boolean),
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
            Some(PortValue::Single(ScalarValue::Json(v))) => Ok(v),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Json),
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Gets a reference to a vector value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port does not hold a vector.
    pub fn get_vector(&self, name: &str) -> Result<&Vec<ScalarValue>, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Vector(items)) => Ok(items),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Vector(ScalarType::Json),
                actual: PortType::from_value(other),
            }),
        }
    }

    /// Gets a reference to a map value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port does not hold a map.
    pub fn get_map(&self, name: &str) -> Result<&HashMap<String, ScalarValue>, PortError> {
        match self.0.get(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Map(entries)) => Ok(entries),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Map(ScalarType::Json),
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
        self.0.remove(name).ok_or_else(|| PortError::Missing {
            name: name.to_owned(),
        })
    }

    /// Removes and returns a text value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-text value.
    pub fn take_text(&mut self, name: &str) -> Result<String, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Text(s))) => Ok(s),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Text),
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Removes and returns a number value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-number value.
    pub fn take_number(&mut self, name: &str) -> Result<f64, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Number(n))) => Ok(n),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Number),
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Removes and returns a boolean value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port holds a non-boolean value.
    pub fn take_boolean(&mut self, name: &str) -> Result<bool, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Single(ScalarValue::Boolean(b))) => Ok(b),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Boolean),
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
            Some(PortValue::Single(ScalarValue::Json(v))) => Ok(v),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Single(ScalarType::Json),
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Removes and returns a vector value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port does not hold a vector.
    pub fn take_vector(&mut self, name: &str) -> Result<Vec<ScalarValue>, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Vector(items)) => Ok(items),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Vector(ScalarType::Json),
                actual: PortType::from_value(&other),
            }),
        }
    }

    /// Removes and returns a map value by port name.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::Missing`] if the port is absent.
    /// Returns [`PortError::TypeMismatch`] if the port does not hold a map.
    pub fn take_map(&mut self, name: &str) -> Result<HashMap<String, ScalarValue>, PortError> {
        match self.0.remove(name) {
            None => Err(PortError::Missing {
                name: name.to_owned(),
            }),
            Some(PortValue::Map(entries)) => Ok(entries),
            Some(other) => Err(PortError::TypeMismatch {
                name: name.to_owned(),
                expected: PortType::Map(ScalarType::Json),
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
    /// A container had mixed scalar types.
    #[display("heterogeneous container: mixed scalar types")]
    HeterogeneousContainer,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing, clippy::expect_used, reason = "test code")]
    use super::*;

    // --- ScalarType::from_value tests ---

    #[test]
    fn scalar_type_from_text_value_returns_text() {
        // Given a text scalar value.
        let value = ScalarValue::Text("hello".to_owned());
        // When getting its scalar type.
        // Then it is Text.
        assert_eq!(ScalarType::from_value(&value), ScalarType::Text);
    }

    #[test]
    fn scalar_type_from_number_value_returns_number() {
        // Given a number scalar value.
        let value = ScalarValue::Number(42.0);
        // When getting its scalar type.
        // Then it is Number.
        assert_eq!(ScalarType::from_value(&value), ScalarType::Number);
    }

    #[test]
    fn scalar_type_from_boolean_value_returns_boolean() {
        // Given a boolean scalar value.
        let value = ScalarValue::Boolean(true);
        // When getting its scalar type.
        // Then it is Boolean.
        assert_eq!(ScalarType::from_value(&value), ScalarType::Boolean);
    }

    #[test]
    fn scalar_type_from_json_value_returns_json() {
        // Given a JSON scalar value.
        let value = ScalarValue::Json(serde_json::json!(null));
        // When getting its scalar type.
        // Then it is Json.
        assert_eq!(ScalarType::from_value(&value), ScalarType::Json);
    }

    // --- ScalarValue extraction tests ---

    #[test]
    fn into_text_returns_inner_string() {
        // Given a text scalar value.
        let value = ScalarValue::Text("hello".to_owned());
        // When extracting as text.
        let result = value.into_text();
        // Then it returns the inner string.
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn into_text_returns_type_mismatch_for_number() {
        // Given a number scalar value.
        let value = ScalarValue::Number(42.0);
        // When extracting as text.
        let result = value.into_text();
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn into_number_returns_inner_f64() {
        // Given a number scalar value.
        let value = ScalarValue::Number(3.15);
        // When extracting as number.
        let result = value.into_number();
        // Then it returns the inner f64.
        assert!((result.unwrap() - 3.15).abs() < f64::EPSILON);
    }

    #[test]
    fn into_number_returns_type_mismatch_for_text() {
        // Given a text scalar value.
        let value = ScalarValue::Text("hello".to_owned());
        // When extracting as number.
        let result = value.into_number();
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn into_boolean_returns_inner_bool() {
        // Given a boolean scalar value.
        let value = ScalarValue::Boolean(true);
        // When extracting as boolean.
        let result = value.into_boolean();
        // Then it returns the inner bool.
        assert!(result.unwrap());
    }

    #[test]
    fn into_boolean_returns_type_mismatch_for_text() {
        // Given a text scalar value.
        let value = ScalarValue::Text("hello".to_owned());
        // When extracting as boolean.
        let result = value.into_boolean();
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn into_json_returns_inner_value() {
        // Given a JSON scalar value.
        let json = serde_json::json!({"key": 42});
        let value = ScalarValue::Json(json.clone());
        // When extracting as JSON.
        let result = value.into_json();
        // Then it returns the inner value.
        assert_eq!(result.unwrap(), json);
    }

    #[test]
    fn into_json_returns_type_mismatch_for_text() {
        // Given a text scalar value.
        let value = ScalarValue::Text("hello".to_owned());
        // When extracting as JSON.
        let result = value.into_json();
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    // --- PortValue construction tests ---

    #[test]
    fn vector_with_homogeneous_numbers_succeeds() {
        // Given a list of number scalars.
        let items = vec![ScalarValue::Number(1.0), ScalarValue::Number(2.0)];
        // When constructing a vector.
        let result = PortValue::vector(items);
        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn vector_with_empty_items_succeeds() {
        // Given an empty list.
        // When constructing a vector.
        let result = PortValue::vector(vec![]);
        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn vector_with_mixed_types_returns_err() {
        // Given a list with text and number scalars.
        let items = vec![ScalarValue::Text("a".to_owned()), ScalarValue::Number(42.0)];
        // When constructing a vector.
        let result = PortValue::vector(items);
        // Then it returns HeterogeneousContainer.
        assert!(matches!(result, Err(PortError::HeterogeneousContainer)));
    }

    #[test]
    fn map_with_homogeneous_text_succeeds() {
        // Given a map with text values.
        let mut entries = HashMap::new();
        entries.insert("name".to_owned(), ScalarValue::Text("alice".to_owned()));
        entries.insert("city".to_owned(), ScalarValue::Text("NYC".to_owned()));
        // When constructing a map.
        let result = PortValue::map(entries);
        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn map_with_empty_entries_succeeds() {
        // Given an empty map.
        // When constructing a map.
        let result = PortValue::map(HashMap::new());
        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn map_with_mixed_types_returns_err() {
        // Given a map with text and number values.
        let mut entries = HashMap::new();
        entries.insert("name".to_owned(), ScalarValue::Text("alice".to_owned()));
        entries.insert("age".to_owned(), ScalarValue::Number(30.0));
        // When constructing a map.
        let result = PortValue::map(entries);
        // Then it returns HeterogeneousContainer.
        assert!(matches!(result, Err(PortError::HeterogeneousContainer)));
    }

    // --- PortDef convenience constructor tests ---

    #[test]
    fn port_def_text_creates_single_text_port() {
        // Given a name for a text port.
        let def = PortDef::text("prompt");
        // When creating via convenience method.
        // Then it has Single(Text) type.
        assert_eq!(def.name, "prompt");
        assert_eq!(def.value_type, PortType::Single(ScalarType::Text));
    }

    #[test]
    fn port_def_number_creates_single_number_port() {
        // Given a name for a number port.
        let def = PortDef::number("count");
        // When creating via convenience method.
        // Then it has Single(Number) type.
        assert_eq!(def.name, "count");
        assert_eq!(def.value_type, PortType::Single(ScalarType::Number));
    }

    #[test]
    fn port_def_boolean_creates_single_boolean_port() {
        // Given a name for a boolean port.
        let def = PortDef::boolean("flag");
        // When creating via convenience method.
        // Then it has Single(Boolean) type.
        assert_eq!(def.name, "flag");
        assert_eq!(def.value_type, PortType::Single(ScalarType::Boolean));
    }

    #[test]
    fn port_def_json_creates_single_json_port() {
        // Given a name for a JSON port.
        let def = PortDef::json("data");
        // When creating via convenience method.
        // Then it has Single(Json) type.
        assert_eq!(def.name, "data");
        assert_eq!(def.value_type, PortType::Single(ScalarType::Json));
    }

    #[test]
    fn port_def_vec_of_creates_vector_port() {
        // Given a name and element type.
        let def = PortDef::vec_of("items", ScalarType::Number);
        // When creating via convenience method.
        // Then it has Vector(Number) type.
        assert_eq!(def.name, "items");
        assert_eq!(def.value_type, PortType::Vector(ScalarType::Number));
    }

    #[test]
    fn port_def_map_of_creates_map_port() {
        // Given a name and value type.
        let def = PortDef::map_of("headers", ScalarType::Text);
        // When creating via convenience method.
        // Then it has Map(Text) type.
        assert_eq!(def.name, "headers");
        assert_eq!(def.value_type, PortType::Map(ScalarType::Text));
    }

    // --- PortType::from_value tests ---

    #[test]
    fn port_type_from_single_text_returns_single_text() {
        // Given a single text value.
        let value = PortValue::Single(ScalarValue::Text("hi".to_owned()));
        // When getting its port type.
        let result = PortType::from_value(&value);
        // Then it is Single(Text).
        assert_eq!(result, PortType::Single(ScalarType::Text));
    }

    #[test]
    fn port_type_from_single_number_returns_single_number() {
        // Given a single number value.
        let value = PortValue::Single(ScalarValue::Number(42.0));
        // When getting its port type.
        let result = PortType::from_value(&value);
        // Then it is Single(Number).
        assert_eq!(result, PortType::Single(ScalarType::Number));
    }

    #[test]
    fn port_type_from_vector_numbers_returns_vector_number() {
        // Given a vector of numbers.
        let value =
            PortValue::vector(vec![ScalarValue::Number(1.0), ScalarValue::Number(2.0)]).unwrap();
        // When getting its port type.
        let result = PortType::from_value(&value);
        // Then it is Vector(Number).
        assert_eq!(result, PortType::Vector(ScalarType::Number));
    }

    #[test]
    fn port_type_from_empty_vector_defaults_to_json() {
        // Given an empty vector.
        let value = PortValue::vector(vec![]).unwrap();
        // When getting its port type.
        let result = PortType::from_value(&value);
        // Then it defaults to Vector(Json).
        assert_eq!(result, PortType::Vector(ScalarType::Json));
    }

    #[test]
    fn port_type_from_empty_map_defaults_to_json() {
        // Given an empty map.
        let value = PortValue::map(HashMap::new()).unwrap();
        // When getting its port type.
        let result = PortType::from_value(&value);
        // Then it defaults to Map(Json).
        assert_eq!(result, PortType::Map(ScalarType::Json));
    }

    // --- PortValues accessor tests ---

    #[test]
    fn take_text_returns_value_when_present() {
        // Given a PortValues with a text port.
        let mut values = PortValues::new();
        values.insert(
            "prompt".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        // When taking the text value.
        let result = values.take_text("prompt");
        // Then it returns the value.
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn take_text_returns_missing_when_absent() {
        // Given an empty PortValues.
        let mut values = PortValues::new();
        // When taking a non-existent port.
        let result = values.take_text("missing");
        // Then it returns Missing error.
        assert!(matches!(result, Err(PortError::Missing { .. })));
    }

    #[test]
    fn take_text_returns_type_mismatch_for_number_value() {
        // Given a PortValues with a number port.
        let mut values = PortValues::new();
        values.insert(
            "count".to_owned(),
            PortValue::Single(ScalarValue::Number(42.0)),
        );
        // When taking as text.
        let result = values.take_text("count");
        // Then it returns TypeMismatch error.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn take_text_returns_type_mismatch_for_vector_value() {
        // Given a PortValues with a vector port.
        let mut values = PortValues::new();
        values.insert(
            "items".to_owned(),
            PortValue::vector(vec![ScalarValue::Number(1.0)]).unwrap(),
        );
        // When taking as text.
        let result = values.take_text("items");
        // Then it returns TypeMismatch error.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn take_number_returns_value_when_present() {
        // Given a PortValues with a number port.
        let mut values = PortValues::new();
        values.insert(
            "count".to_owned(),
            PortValue::Single(ScalarValue::Number(42.0)),
        );
        // When taking the number value.
        let result = values.take_number("count");
        // Then it returns the value.
        assert!((result.unwrap() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn take_boolean_returns_value_when_present() {
        // Given a PortValues with a boolean port.
        let mut values = PortValues::new();
        values.insert(
            "flag".to_owned(),
            PortValue::Single(ScalarValue::Boolean(true)),
        );
        // When taking the boolean value.
        let result = values.take_boolean("flag");
        // Then it returns the value.
        assert!(result.unwrap());
    }

    #[test]
    fn take_json_returns_value_when_present() {
        // Given a PortValues with a JSON port.
        let mut values = PortValues::new();
        let json = serde_json::json!({"key": 42});
        values.insert(
            "data".to_owned(),
            PortValue::Single(ScalarValue::Json(json.clone())),
        );
        // When taking the JSON value.
        let result = values.take_json("data");
        // Then it returns the value.
        assert_eq!(result.unwrap(), json);
    }

    #[test]
    fn take_vector_returns_value_when_present() {
        // Given a PortValues with a vector port.
        let mut values = PortValues::new();
        values.insert(
            "items".to_owned(),
            PortValue::vector(vec![ScalarValue::Number(1.0), ScalarValue::Number(2.0)]).unwrap(),
        );
        // When taking the vector value.
        let result = values.take_vector("items");
        // Then it returns the items.
        let items = result.unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn take_vector_returns_type_mismatch_for_single() {
        // Given a PortValues with a single text port.
        let mut values = PortValues::new();
        values.insert(
            "name".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        // When taking as vector.
        let result = values.take_vector("name");
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn take_map_returns_value_when_present() {
        // Given a PortValues with a map port.
        let mut values = PortValues::new();
        let mut entries = HashMap::new();
        entries.insert("key".to_owned(), ScalarValue::Text("value".to_owned()));
        values.insert("headers".to_owned(), PortValue::map(entries).unwrap());
        // When taking the map value.
        let result = values.take_map("headers");
        // Then it returns the entries.
        assert!(result.unwrap().contains_key("key"));
    }

    #[test]
    fn take_map_returns_type_mismatch_for_single() {
        // Given a PortValues with a single text port.
        let mut values = PortValues::new();
        values.insert(
            "name".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        // When taking as map.
        let result = values.take_map("name");
        // Then it returns TypeMismatch.
        assert!(matches!(result, Err(PortError::TypeMismatch { .. })));
    }

    #[test]
    fn get_text_returns_reference_to_value() {
        // Given a PortValues with a text port.
        let mut values = PortValues::new();
        values.insert(
            "prompt".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        // When getting the text reference.
        let result = values.get_text("prompt");
        // Then it returns a reference to the value.
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn get_json_returns_reference_to_value() {
        // Given a PortValues with a JSON port.
        let mut values = PortValues::new();
        let json = serde_json::json!({"key": "value"});
        values.insert(
            "data".to_owned(),
            PortValue::Single(ScalarValue::Json(json.clone())),
        );
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

    // --- PortValues round-trip and container tests ---

    #[test]
    fn insert_and_take_roundtrip_text() {
        // Given a PortValues with a text port.
        let mut values = PortValues::new();
        values.insert(
            "text".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        // When taking the value back.
        let text = values.take_text("text").unwrap();
        // Then the value is recovered and the key is gone.
        assert_eq!(text, "hello");
        assert!(!values.contains("text"));
    }

    #[test]
    fn insert_and_take_roundtrip_json() {
        // Given a PortValues with a JSON port.
        let mut values = PortValues::new();
        values.insert(
            "data".to_owned(),
            PortValue::Single(ScalarValue::Json(serde_json::json!([1, 2, 3]))),
        );
        // When taking the value back.
        let data = values.take_json("data").unwrap();
        // Then the value is recovered and the key is gone.
        assert_eq!(data, serde_json::json!([1, 2, 3]));
        assert!(!values.contains("data"));
    }

    #[test]
    fn contains_returns_true_for_present_port() {
        // Given a PortValues with a port.
        let mut values = PortValues::new();
        values.insert(
            "name".to_owned(),
            PortValue::Single(ScalarValue::Text("test".to_owned())),
        );
        // When checking if the port exists.
        // Then it returns true for the present port and false for absent.
        assert!(values.contains("name"));
        assert!(!values.contains("other"));
    }

    #[test]
    fn len_returns_number_of_ports() {
        // Given a PortValues with two ports.
        let mut values = PortValues::new();
        values.insert(
            "a".to_owned(),
            PortValue::Single(ScalarValue::Text("1".to_owned())),
        );
        values.insert(
            "b".to_owned(),
            PortValue::Single(ScalarValue::Text("2".to_owned())),
        );
        // When checking length.
        // Then it returns 2.
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn iter_yields_all_port_pairs() {
        // Given a PortValues with two ports.
        let mut values = PortValues::new();
        values.insert(
            "a".to_owned(),
            PortValue::Single(ScalarValue::Text("1".to_owned())),
        );
        values.insert("b".to_owned(), PortValue::Single(ScalarValue::Number(2.0)));
        // When iterating.
        let pairs: Vec<_> = values.iter().collect();
        // Then both pairs are present.
        assert_eq!(pairs.len(), 2);
    }

    // --- From trait tests ---

    #[test]
    fn from_str_creates_text_port_value() {
        // Given a &str.
        let value: PortValue = "hello".into();
        // When converting to PortValue.
        // Then it is a Single(Text) variant.
        assert_eq!(
            value,
            PortValue::Single(ScalarValue::Text("hello".to_owned()))
        );
    }

    #[test]
    fn from_string_creates_text_port_value() {
        // Given a String.
        let value: PortValue = "hello".to_owned().into();
        // When converting to PortValue.
        // Then it is a Single(Text) variant.
        assert_eq!(
            value,
            PortValue::Single(ScalarValue::Text("hello".to_owned()))
        );
    }

    #[test]
    fn from_f64_creates_number_port_value() {
        // Given an f64.
        let value: PortValue = 42.0f64.into();
        // When converting to PortValue.
        // Then it is a Single(Number) variant.
        assert_eq!(value, PortValue::Single(ScalarValue::Number(42.0)));
    }

    #[test]
    fn from_bool_creates_boolean_port_value() {
        // Given a bool.
        let value: PortValue = true.into();
        // When converting to PortValue.
        // Then it is a Single(Boolean) variant.
        assert_eq!(value, PortValue::Single(ScalarValue::Boolean(true)));
    }

    #[test]
    fn from_json_value_creates_json_port_value() {
        // Given a serde_json::Value.
        let json = serde_json::json!({"key": 42});
        let value: PortValue = json.clone().into();
        // When converting to PortValue.
        // Then it is a Single(Json) variant.
        assert_eq!(value, PortValue::Single(ScalarValue::Json(json)));
    }

    // --- port_type() method tests ---

    #[test]
    fn port_value_port_type_returns_correct_type() {
        // Given text and number values.
        let text_val = PortValue::Single(ScalarValue::Text("test".to_owned()));
        let number_val = PortValue::Single(ScalarValue::Number(42.0));
        // When getting their port types.
        // Then they match their variants.
        assert_eq!(text_val.port_type(), PortType::Single(ScalarType::Text));
        assert_eq!(number_val.port_type(), PortType::Single(ScalarType::Number));
    }

    // --- Mutant-killing tests for port.rs ---

    // Kills: get_number -> Ok(0.0), Ok(1.0), Ok(-1.0)
    #[test]
    fn get_number_returns_actual_stored_value() {
        let mut values = PortValues::new();
        values.insert(
            "ratio".to_owned(),
            PortValue::Single(ScalarValue::Number(3.7)),
        );
        let result = values.get_number("ratio");
        let n = result.expect("should find number");
        assert!((n - 3.7).abs() < f64::EPSILON, "expected 3.7, got {n}");
        assert!(n != 0.0, "must not be 0.0");
        assert!(n != 1.0, "must not be 1.0");
        assert!(n != -1.0, "must not be -1.0");
    }

    // Kills: get_boolean -> Ok(true), Ok(false)
    #[test]
    fn get_boolean_returns_actual_false_value() {
        let mut values = PortValues::new();
        values.insert(
            "flag".to_owned(),
            PortValue::Single(ScalarValue::Boolean(false)),
        );
        let result = values.get_boolean("flag");
        assert!(!result.unwrap(), "must return the stored false, not true");
    }

    #[test]
    fn get_boolean_returns_actual_true_value() {
        let mut values = PortValues::new();
        values.insert(
            "flag".to_owned(),
            PortValue::Single(ScalarValue::Boolean(true)),
        );
        let result = values.get_boolean("flag");
        assert!(result.unwrap(), "must return the stored true, not false");
    }

    // Kills: get_vector -> Ok(Box::leak(Box::new(vec![])))
    #[test]
    fn get_vector_returns_actual_vector_contents() {
        let mut values = PortValues::new();
        let items = vec![
            ScalarValue::Number(10.0),
            ScalarValue::Number(20.0),
            ScalarValue::Number(30.0),
        ];
        values.insert("data".to_owned(), PortValue::Vector(items));
        let result = values.get_vector("data");
        let v = result.expect("should find vector");
        assert_eq!(v.len(), 3, "must return actual vector, not empty");
        assert!((v[0].clone().into_number().unwrap() - 10.0).abs() < f64::EPSILON);
        assert!((v[2].clone().into_number().unwrap() - 30.0).abs() < f64::EPSILON);
    }

    // Kills: get_map -> Ok(Box::leak(Box::new(HashMap::new())))
    #[test]
    fn get_map_returns_actual_map_contents() {
        let mut values = PortValues::new();
        let mut entries = HashMap::new();
        entries.insert("x".to_owned(), ScalarValue::Number(1.0));
        entries.insert("y".to_owned(), ScalarValue::Number(2.0));
        values.insert("coords".to_owned(), PortValue::Map(entries));
        let result = values.get_map("coords");
        let m = result.expect("should find map");
        assert_eq!(m.len(), 2, "must return actual map, not empty");
        assert!(m.contains_key("x"));
        assert!(m.contains_key("y"));
    }

    // Kills: take_boolean -> Ok(true)
    #[test]
    fn take_boolean_returns_actual_false_value() {
        let mut values = PortValues::new();
        values.insert(
            "flag".to_owned(),
            PortValue::Single(ScalarValue::Boolean(false)),
        );
        let result = values.take_boolean("flag");
        assert!(!result.unwrap(), "must return the stored false, not true");
    }

    // Kills: is_empty -> true, is_empty -> false
    #[test]
    fn is_empty_returns_true_for_empty_port_values() {
        let values = PortValues::new();
        assert!(
            values.is_empty(),
            "empty PortValues must report is_empty=true"
        );
    }

    #[test]
    fn is_empty_returns_false_for_non_empty_port_values() {
        let mut values = PortValues::new();
        values.insert(
            "a".to_owned(),
            PortValue::Single(ScalarValue::Text("x".to_owned())),
        );
        assert!(
            !values.is_empty(),
            "non-empty PortValues must report is_empty=false"
        );
    }

    // Kills: From<HashMap<String, PortValue>> for PortValues -> Default::default()
    #[test]
    fn from_hashmap_preserves_all_entries() {
        let mut map = HashMap::new();
        map.insert(
            "a".to_owned(),
            PortValue::Single(ScalarValue::Text("hello".to_owned())),
        );
        map.insert("b".to_owned(), PortValue::Single(ScalarValue::Number(42.0)));
        map.insert(
            "c".to_owned(),
            PortValue::Single(ScalarValue::Boolean(true)),
        );
        let values: PortValues = map.into();
        assert_eq!(values.len(), 3, "From<HashMap> must preserve all 3 entries");
        assert_eq!(values.get_text("a").unwrap(), "hello");
        assert!((values.get_number("b").unwrap() - 42.0).abs() < f64::EPSILON);
        assert!(values.get_boolean("c").unwrap());
    }
}
