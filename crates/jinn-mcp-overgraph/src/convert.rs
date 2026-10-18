//! Pure JSON ⇄ engine value converters.
//!
//! Two directions, both total over JSON's shapes:
//!
//! - JSON → [`GqlParamValue`] (query parameters) and JSON → [`PropValue`]
//!   (upsert properties). JSON has no byte string, so the engine's `Bytes`
//!   variant is unreachable from this direction.
//! - [`GqlValue`] → JSON (query results). Graph payloads (`Node`, `Edge`,
//!   `Path`) render as their structured JSON forms; `Bytes` renders as a
//!   base64 string so the roundtrip never invents data.

use std::collections::BTreeMap;

use overgraph::types::{GqlExecutionResult, GqlParamValue, GqlValue, PropValue};
use serde_json::{Number, Value, json};

/// The error a JSON → engine-value conversion can fail with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertError(pub String);

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Converts a JSON value into a GQL parameter value.
///
/// Numbers: integers become [`GqlParamValue::Int`] (or [`GqlParamValue::UInt`]
/// beyond `i64`'s positive range), everything else numeric becomes
/// [`GqlParamValue::Float`]. Arrays and objects convert recursively.
///
/// # Errors
///
/// Infallible over JSON's shapes; `ConvertError` exists to keep the signature
/// honest for callers mapping per-key failures.
pub fn json_to_gql_param_value(value: &Value) -> Result<GqlParamValue, ConvertError> {
    json_to_prop_value(value).map(prop_to_param)
}

/// Converts a JSON value into a node/edge property value.
///
/// Same number rules as [`json_to_gql_param_value`].
///
/// # Errors
///
/// Infallible over JSON's shapes; see [`json_to_gql_param_value`].
pub fn json_to_prop_value(value: &Value) -> Result<PropValue, ConvertError> {
    let converted = match value {
        Value::Null => PropValue::Null,
        Value::Bool(b) => PropValue::Bool(*b),
        Value::Number(n) => json_number_to_prop(n),
        Value::String(s) => PropValue::String(s.clone()),
        Value::Array(items) => PropValue::Array(
            items
                .iter()
                .map(json_to_prop_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Object(map) => PropValue::Map(json_map_to_prop_map(map).map_err(|(_, e)| e)?),
    };
    Ok(converted)
}

/// Converts a JSON object into the engine's property map.
///
/// Returns the offending key alongside the error so callers can point at the
/// exact property that failed.
///
/// # Errors
///
/// Infallible over JSON's shapes; see [`json_to_prop_value`].
pub fn json_map_to_prop_map(
    map: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, PropValue>, (String, ConvertError)> {
    let mut out = BTreeMap::new();
    for (key, value) in map {
        let prop = json_to_prop_value(value).map_err(|e| (key.clone(), e))?;
        out.insert(key.clone(), prop);
    }
    Ok(out)
}

/// Converts an engine result value back into JSON for tool output.
///
/// UInts too large for `i64` pass through as JSON numbers (serde_json carries
/// `u64` natively); `Bytes` renders as base64; graph payloads render as
/// structured objects.
#[must_use]
pub fn gql_value_to_json(value: &GqlValue) -> Value {
    match value {
        GqlValue::Null => Value::Null,
        GqlValue::Bool(b) => Value::Bool(*b),
        GqlValue::Int(i) => Number::from(*i).into(),
        GqlValue::UInt(u) => Number::from(*u).into(),
        GqlValue::Float(f) => match Number::from_f64(*f) {
            Some(number) => Value::Number(number),
            // NaN/∞ have no JSON form; Null keeps the conversion total.
            None => Value::Null,
        },
        GqlValue::String(s) => Value::String(s.clone()),
        GqlValue::Bytes(bytes) => Value::String(base64_std_encode(bytes)),
        GqlValue::List(items) => Value::Array(items.iter().map(gql_value_to_json).collect()),
        GqlValue::Map(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), gql_value_to_json(v)))
                .collect(),
        ),
        GqlValue::Node(node) => json!({
            "id": node.id,
            "labels": node.labels,
            "key": node.key,
            "props": node.props.as_ref().map(gql_prop_map_to_json),
        }),
        GqlValue::Edge(edge) => json!({
            "id": edge.id,
            "from": edge.from,
            "to": edge.to,
            "label": edge.label,
            "props": edge.props.as_ref().map(gql_prop_map_to_json),
        }),
        GqlValue::Path(path) => json!({
            "node_ids": path.node_ids,
            "edge_ids": path.edge_ids,
        }),
    }
}

/// Serializes a full GQL execution result: columns zipped with each row's
/// positional values, plus cursor, warnings, and mutation counts when present.
#[must_use]
pub fn gql_execution_result_to_json(result: &GqlExecutionResult) -> Value {
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|row| {
            let fields: serde_json::Map<String, Value> = result
                .columns
                .iter()
                .zip(row.values.iter())
                .map(|(column, value)| (column.clone(), gql_value_to_json(value)))
                .collect();
            Value::Object(fields)
        })
        .collect();

    let mut out = serde_json::Map::new();
    out.insert(
        "kind".into(),
        json!(format!("{:?}", result.kind).to_lowercase()),
    );
    out.insert("columns".into(), json!(result.columns));
    out.insert("rows".into(), Value::Array(rows));
    out.insert("rows_returned".into(), json!(result.stats.rows_returned));
    if !result.stats.warnings.is_empty() {
        out.insert("warnings".into(), json!(result.stats.warnings));
    }
    if let Some(cursor) = &result.next_cursor {
        out.insert("next_cursor".into(), json!(cursor));
    }
    if let Some(mutation) = &result.mutation_stats {
        out.insert(
            "mutations".into(),
            json!({
                "nodes_created": mutation.nodes_created,
                "nodes_updated": mutation.nodes_updated,
                "nodes_deleted": mutation.nodes_deleted,
                "edges_created": mutation.edges_created,
                "edges_updated": mutation.edges_updated,
                "edges_deleted": mutation.edges_deleted,
                "properties_set": mutation.properties_set,
                "properties_removed": mutation.properties_removed,
            }),
        );
    }
    Value::Object(out)
}

/// Converts an engine property map into a JSON object (tool-output direction).
///
/// Used by [`crate::server`] to enrich vector-search hits with node props.
#[must_use]
pub fn prop_map_to_json(map: &BTreeMap<String, PropValue>) -> Value {
    Value::Object(
        map.iter()
            .map(|(key, prop)| (key.clone(), prop_value_to_json(prop)))
            .collect(),
    )
}

/// Converts one engine property value into JSON.
#[must_use]
pub fn prop_value_to_json(prop: &PropValue) -> Value {
    match prop {
        PropValue::Null => Value::Null,
        PropValue::Bool(b) => Value::Bool(*b),
        PropValue::Int(i) => Number::from(*i).into(),
        PropValue::UInt(u) => Number::from(*u).into(),
        PropValue::Float(f) => match Number::from_f64(*f) {
            Some(number) => Value::Number(number),
            // NaN/∞ have no JSON form; Null keeps the conversion total.
            None => Value::Null,
        },
        PropValue::String(s) => Value::String(s.clone()),
        PropValue::Bytes(bytes) => Value::String(base64_std_encode(bytes)),
        PropValue::Array(items) => Value::Array(items.iter().map(prop_value_to_json).collect()),
        PropValue::Map(map) => prop_map_to_json(map),
    }
}

fn json_number_to_prop(n: &Number) -> PropValue {
    if let Some(i) = n.as_i64() {
        PropValue::Int(i)
    } else if let Some(u) = n.as_u64() {
        PropValue::UInt(u)
    } else {
        PropValue::Float(n.as_f64().unwrap_or(f64::NAN))
    }
}

fn prop_to_param(prop: PropValue) -> GqlParamValue {
    match prop {
        PropValue::Null => GqlParamValue::Null,
        PropValue::Bool(b) => GqlParamValue::Bool(b),
        PropValue::Int(i) => GqlParamValue::Int(i),
        PropValue::UInt(u) => GqlParamValue::UInt(u),
        PropValue::Float(f) => GqlParamValue::Float(f),
        PropValue::String(s) => GqlParamValue::String(s),
        PropValue::Bytes(b) => GqlParamValue::Bytes(b),
        PropValue::Array(items) => {
            GqlParamValue::List(items.into_iter().map(prop_to_param).collect())
        }
        PropValue::Map(map) => GqlParamValue::Map(
            map.into_iter()
                .map(|(k, v)| (k, prop_to_param(v)))
                .collect(),
        ),
    }
}

fn gql_prop_map_to_json(map: &BTreeMap<String, GqlValue>) -> Value {
    Value::Object(
        map.iter()
            .map(|(k, v)| (k.clone(), gql_value_to_json(v)))
            .collect(),
    )
}

/// Standard (with padding) base64 — a private stand-in so this crate does not
/// pull the `base64` dependency for one rendering path.
fn base64_std_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let encode6 = |six_bits: u32| -> char {
        // 6-bit values are always < 64; `unwrap_or` only silences the
        // out-of-range branch the compiler cannot see.
        ALPHABET
            .get(usize::try_from(six_bits).unwrap_or(63))
            .copied()
            .unwrap_or(b'?') as char
    };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let first = chunk.first().copied().unwrap_or_default();
        let second = chunk.get(1).copied().unwrap_or_default();
        let third = chunk.get(2).copied().unwrap_or_default();
        let n = u32::from_be_bytes([0, first, second, third]);
        out.push(encode6(n >> 18));
        out.push(encode6((n >> 12) & 0x3f));
        out.push(if chunk.len() > 1 {
            encode6((n >> 6) & 0x3f)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            encode6(n & 0x3f)
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code asserts with expect/panic and indexes JSON fixtures for clear failures"
)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case(json!(null), PropValue::Null)]
    #[case(json!(true), PropValue::Bool(true))]
    #[case(json!(false), PropValue::Bool(false))]
    #[case(json!(42), PropValue::Int(42))]
    #[case(json!(-7), PropValue::Int(-7))]
    #[case(json!(18_446_744_073_709_551_615u64), PropValue::UInt(18_446_744_073_709_551_615))]
    #[case(json!(1.5), PropValue::Float(1.5))]
    #[case(json!("hello"), PropValue::String("hello".into()))]
    fn json_converts_to_prop_value_across_scalar_shapes(
        #[case] input: serde_json::Value,
        #[case] expected: PropValue,
    ) {
        // Given a JSON scalar of one shape.
        // When converting to a property value.
        let converted = json_to_prop_value(&input).expect("conversion is total over JSON");

        // Then it lands in the matching engine variant.
        assert_eq!(converted, expected);
    }

    #[rstest::rstest]
    fn json_converts_nested_arrays_and_maps() {
        // Given a JSON object with nested arrays and maps.
        let input = json!({"tags": ["a", "b"], "meta": {"depth": 2}});

        // When converting to a property value.
        let converted = json_to_prop_value(&input).expect("total over JSON");

        // Then the nesting is preserved structurally.
        assert!(matches!(converted, PropValue::Map(_)));
        let PropValue::Map(map) = converted else {
            panic!("expected a map");
        };
        assert!(matches!(map.get("tags"), Some(PropValue::Array(_))));
    }

    #[rstest::rstest]
    #[case(json!(42), GqlParamValue::Int(42))]
    #[case(json!("x"), GqlParamValue::String("x".into()))]
    #[case(json!([1, 2]), GqlParamValue::List(vec![GqlParamValue::Int(1), GqlParamValue::Int(2)]))]
    fn json_converts_to_gql_param_value(
        #[case] input: serde_json::Value,
        #[case] expected: GqlParamValue,
    ) {
        // Given a JSON value used as a query parameter.
        // When converting.
        let converted = json_to_gql_param_value(&input).expect("conversion is total over JSON");

        // Then it lands in the matching parameter variant.
        assert_eq!(converted, expected);
    }

    #[rstest::rstest]
    fn json_map_conversion_preserves_every_key() {
        // Given a JSON map with mixed value shapes.
        let map = json!({"a": 1, "b": "x", "c": null})
            .as_object()
            .cloned()
            .expect("object");

        // When converting the map.
        let converted = json_map_to_prop_map(&map).expect("total over JSON");

        // Then every key survives with its converted value.
        assert_eq!(converted.len(), 3);
        assert_eq!(converted.get("a"), Some(&PropValue::Int(1)));
        assert_eq!(converted.get("b"), Some(&PropValue::String("x".into())));
        assert_eq!(converted.get("c"), Some(&PropValue::Null));
    }

    #[rstest::rstest]
    fn gql_value_renders_nodes_as_structured_json() {
        // Given a graph node value.
        let node = GqlValue::Node(overgraph::types::GqlNode {
            id: Some(7),
            labels: Some(vec!["Person".into()]),
            key: Some("alice".into()),
            props: Some(
                [("age".to_owned(), GqlValue::Int(30))]
                    .into_iter()
                    .collect(),
            ),
            weight: None,
            created_at: None,
            updated_at: None,
            dense_vector: None,
            sparse_vector: None,
        });

        // When rendering to JSON.
        let rendered = gql_value_to_json(&node);

        // Then the structured shape survives.
        assert_eq!(rendered["id"], json!(7));
        assert_eq!(rendered["labels"], json!(["Person"]));
        assert_eq!(rendered["key"], json!("alice"));
        assert_eq!(rendered["props"]["age"], json!(30));
    }

    #[rstest::rstest]
    fn gql_value_renders_edges_with_endpoints() {
        // Given a graph edge value.
        let edge = GqlValue::Edge(overgraph::types::GqlEdge {
            id: Some(11),
            from: Some(1),
            to: Some(2),
            label: Some("KNOWS".into()),
            props: None,
            weight: Some(1.5),
            created_at: None,
            updated_at: None,
            valid_from: None,
            valid_to: None,
        });

        // When rendering to JSON.
        let rendered = gql_value_to_json(&edge);

        // Then endpoints and label survive.
        assert_eq!(rendered["id"], json!(11));
        assert_eq!(rendered["from"], json!(1));
        assert_eq!(rendered["to"], json!(2));
        assert_eq!(rendered["label"], json!("KNOWS"));
    }

    #[rstest::rstest]
    fn gql_value_renders_bytes_as_base64() {
        // Given a byte payload.
        let value = GqlValue::Bytes(vec![0xfb, 0xff]);

        // When rendering to JSON.
        let rendered = gql_value_to_json(&value);

        // Then it is standard base64 with padding.
        assert_eq!(rendered, json!("+/8="));
    }

    #[rstest::rstest]
    fn gql_value_renders_out_of_range_floats_as_null() {
        // Given a float with no JSON form.
        let value = GqlValue::Float(f64::NAN);

        // When rendering to JSON.
        let rendered = gql_value_to_json(&value);

        // Then it renders as null rather than failing.
        assert_eq!(rendered, json!(null));
    }

    #[rstest::rstest]
    fn gql_execution_result_zips_columns_with_row_values() {
        // Given an execution result with two columns and one row.
        let result = GqlExecutionResult {
            kind: overgraph::types::GqlStatementKind::Query,
            columns: vec!["name".into(), "age".into()],
            rows: vec![overgraph::types::GqlRow {
                values: vec![GqlValue::String("alice".into()), GqlValue::Int(30)],
            }],
            next_cursor: None,
            stats: overgraph::types::GqlExecutionStats {
                rows_returned: 1,
                rows_matched: 0,
                rows_after_filter: 0,
                intermediate_bindings: 0,
                db_hits: 0,
                elapsed_us: None,
                warnings: Vec::new(),
            },
            mutation_stats: None,
            schema_stats: None,
            index_stats: None,
            plan: None,
        };

        // When serializing.
        let rendered = gql_execution_result_to_json(&result);

        // Then row objects are keyed by column name, positionally zipped.
        assert_eq!(rendered["columns"], json!(["name", "age"]));
        assert_eq!(rendered["rows"][0]["name"], json!("alice"));
        assert_eq!(rendered["rows"][0]["age"], json!(30));
        assert_eq!(rendered["rows_returned"], json!(1));
        // And absent optional parts stay absent.
        assert!(rendered.get("next_cursor").is_none());
        assert!(rendered.get("warnings").is_none());
        assert!(rendered.get("mutations").is_none());
    }

    #[rstest::rstest]
    fn gql_execution_result_surfaces_cursor_warnings_and_mutations() {
        // Given an execution result with a cursor, a warning, and mutation counts.
        let result = GqlExecutionResult {
            kind: overgraph::types::GqlStatementKind::Mutation,
            columns: vec![],
            rows: vec![],
            next_cursor: Some("cursor-1".into()),
            stats: overgraph::types::GqlExecutionStats {
                rows_returned: 0,
                rows_matched: 0,
                rows_after_filter: 0,
                intermediate_bindings: 0,
                db_hits: 0,
                elapsed_us: None,
                warnings: vec!["watch out".into()],
            },
            mutation_stats: Some(overgraph::types::GqlMutationStats {
                rows_matched: 0,
                mutation_rows: 0,
                mutation_ops: 1,
                nodes_created: 2,
                nodes_updated: 0,
                nodes_deleted: 0,
                edges_created: 0,
                edges_updated: 0,
                edges_deleted: 0,
                labels_added: 0,
                labels_removed: 0,
                properties_set: 3,
                properties_removed: 0,
                skipped_null_targets: 0,
                duplicate_targets: 0,
                db_hits: 0,
                elapsed_us: None,
                warnings: Vec::new(),
            }),
            schema_stats: None,
            index_stats: None,
            plan: None,
        };

        // When serializing.
        let rendered = gql_execution_result_to_json(&result);

        // Then all three surfaces are present.
        assert_eq!(rendered["next_cursor"], json!("cursor-1"));
        assert_eq!(rendered["warnings"], json!(["watch out"]));
        assert_eq!(rendered["mutations"]["nodes_created"], json!(2));
        assert_eq!(rendered["mutations"]["properties_set"], json!(3));
    }
}
