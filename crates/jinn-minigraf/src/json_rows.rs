//! Renders minigraf `QueryResult` rows as JSON for the tool wire.
//!
//! Column names come from the `:find` clause (`vars`); each row is a JSON
//! array aligned with them. Values render by minigraf variant: strings and
//! numbers keep their JSON shape, refs render as their UUID, keywords keep
//! their leading colon, and `nil` is `null`.

use minigraf::{EntityId, QueryResult, Value};

/// Renders a successful query result as a JSON string.
///
/// `{"columns": [...], "rows": [[...], ...]}` — rows are arrays aligned with
/// `columns`, not objects, so a row's shape survives `:find` reordering
/// without key collisions.
#[must_use]
pub fn render(result: &QueryResult) -> String {
    match result {
        QueryResult::QueryResults { vars, results } => {
            let rows: Vec<Vec<serde_json::Value>> = results
                .iter()
                .map(|row| row.iter().map(render_value).collect())
                .collect();
            serde_json::json!({ "columns": vars, "rows": rows }).to_string()
        }
        // A `(query ...)` command always yields `QueryResults`; anything else
        // reaching the query tool is a caller bug, rendered honestly instead
        // of guessed at.
        other => serde_json::json!({ "columns": [], "rows": [], "note": format!("{other:?}") })
            .to_string(),
    }
}

fn render_value(value: &Value) -> serde_json::Value {
    match value {
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Integer(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::json!(f),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Ref(id) => serde_json::Value::String(render_ref(*id)),
        Value::Keyword(k) => serde_json::Value::String(k.clone()),
        Value::Null => serde_json::Value::Null,
    }
}

/// UUIDs render in their canonical hyphenated form — the same shape the
/// `#uuid "..."` Datalog literal accepts, so a query result can be pasted
/// into the next query.
#[must_use]
pub fn render_ref(id: EntityId) -> String {
    id.to_string()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test fixtures and assertions on known shapes"
    )]

    use super::*;

    fn rows_of(json: &str) -> Vec<Vec<serde_json::Value>> {
        let v: serde_json::Value = serde_json::from_str(json).expect("json");
        v["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| r.as_array().expect("row").clone())
            .collect()
    }

    #[rstest::rstest]
    #[test]
    fn renders_string_integer_and_columns() {
        // Given a query result with a string and an integer column.
        let result = QueryResult::QueryResults {
            vars: vec!["name".to_owned(), "age".to_owned()],
            results: vec![vec![Value::String("Alice".to_owned()), Value::Integer(30)]],
        };

        // When rendering.
        let json = render(&result);

        // Then columns and typed rows survive.
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["columns"][0], "name");
        assert_eq!(v["columns"][1], "age");
        assert_eq!(rows_of(&json)[0][0], "Alice");
        assert_eq!(rows_of(&json)[0][1], 30);
    }

    #[rstest::rstest]
    #[test]
    fn renders_ref_keyword_boolean_and_null() {
        // Given a row with a ref, keyword, boolean, and nil.
        let id: EntityId = "550e8400-e29b-41d4-a716-446655440000"
            .parse()
            .expect("uuid");
        let result = QueryResult::QueryResults {
            vars: vec![
                "e".to_owned(),
                "k".to_owned(),
                "b".to_owned(),
                "n".to_owned(),
            ],
            results: vec![vec![
                Value::Ref(id),
                Value::Keyword(":status/active".to_owned()),
                Value::Boolean(true),
                Value::Null,
            ]],
        };

        // When rendering.
        let json = render(&result);

        // Then refs render as UUID strings, keywords keep the colon,
        // booleans stay booleans, and nil is null.
        let row = &rows_of(&json)[0];
        assert_eq!(row[0], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(row[1], ":status/active");
        assert_eq!(row[2], true);
        assert_eq!(row[3], serde_json::Value::Null);
    }

    #[rstest::rstest]
    #[test]
    fn empty_result_renders_empty_rows() {
        // Given a query result with no rows.
        let result = QueryResult::QueryResults {
            vars: vec!["x".to_owned()],
            results: vec![],
        };

        // When rendering.
        let json = render(&result);

        // Then the columns survive and rows is empty (not an error).
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["columns"][0], "x");
        assert!(v["rows"].as_array().expect("rows").is_empty());
    }
}
