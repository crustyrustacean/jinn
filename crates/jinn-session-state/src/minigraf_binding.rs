//! Typed accessors over the session blob map for the minigraf active graph.
//!
//! The active-graph binding is a blob (`integrations.blobs["minigraf.active_graph"]`)
//! so it inherits across forks with the rest of session state and survives
//! persistence without a schema migration. Only jinn's tool-dispatch path
//! reads and writes it — this module is the typed surface over the raw map.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Blob key under `SessionIntegrationFields::blobs` holding the binding.
pub const ACTIVE_GRAPH_BLOB_KEY: &str = "minigraf.active_graph";

/// The session's active minigraf graph: the instance name, the template it
/// came from (none for blank `create_graph` instances), and the session id
/// that activated it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinigrafActiveGraph {
    /// Graph instance name under the minigraf data dir.
    pub name: String,
    /// Template the instance came from, when known.
    #[serde(default)]
    pub template: Option<String>,
    /// Session id that ran the activating tool call.
    #[serde(default)]
    pub created_by: String,
}

/// Reads the active graph from the blob map. A missing, foreign, or
/// unparseable value reads as "no binding" — never an error, never a panic.
#[must_use]
pub fn read_active_graph(blobs: &HashMap<String, JsonValue>) -> Option<MinigrafActiveGraph> {
    blobs
        .get(ACTIVE_GRAPH_BLOB_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Writes (or clears with `None`) the active graph in the blob map.
pub fn write_active_graph(
    blobs: &mut HashMap<String, JsonValue>,
    binding: Option<&MinigrafActiveGraph>,
) {
    match binding {
        Some(binding) => {
            blobs.insert(
                ACTIVE_GRAPH_BLOB_KEY.to_owned(),
                serde_json::to_value(binding).unwrap_or(JsonValue::Null),
            );
        }
        None => {
            blobs.remove(ACTIVE_GRAPH_BLOB_KEY);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test assertions")]

    use super::*;

    fn binding(name: &str) -> MinigrafActiveGraph {
        MinigrafActiveGraph {
            name: name.to_owned(),
            template: Some("stoplight".to_owned()),
            created_by: "session-a".to_owned(),
        }
    }

    #[rstest::rstest]
    #[test]
    fn active_graph_roundtrips_through_the_blob_map() {
        // Given an empty blob map.
        let mut blobs = HashMap::new();

        // When a binding is written and read back.
        write_active_graph(&mut blobs, Some(&binding("sl-a")));
        let read = read_active_graph(&blobs);

        // Then every field survives.
        assert_eq!(read, Some(binding("sl-a")));
    }

    #[rstest::rstest]
    #[test]
    fn foreign_blob_value_reads_as_no_active_graph() {
        // Given the key holding some other subsystem's value.
        let mut blobs = HashMap::new();
        blobs.insert(
            ACTIVE_GRAPH_BLOB_KEY.to_owned(),
            serde_json::json!({ "something": "else entirely" }),
        );

        // When reading the active graph.
        let read = read_active_graph(&blobs);

        // Then there is no binding rather than an error.
        assert_eq!(read, None);
    }

    #[rstest::rstest]
    #[test]
    fn unparseable_blob_value_reads_as_no_active_graph() {
        // Given the key holding a non-object payload.
        let mut blobs = HashMap::new();
        blobs.insert(ACTIVE_GRAPH_BLOB_KEY.to_owned(), JsonValue::Bool(true));

        // When reading the active graph.
        let read = read_active_graph(&blobs);

        // Then there is no binding rather than a panic.
        assert_eq!(read, None);
    }

    #[rstest::rstest]
    #[test]
    fn clearing_removes_the_key_from_the_map() {
        // Given a map with a written binding.
        let mut blobs = HashMap::new();
        write_active_graph(&mut blobs, Some(&binding("sl-a")));

        // When clearing the binding.
        write_active_graph(&mut blobs, None);

        // Then the key is gone entirely.
        assert!(!blobs.contains_key(ACTIVE_GRAPH_BLOB_KEY));
    }
}
