//! Export crossing contracts.
//!
//! The EXPORT surface of the export slice: the command that asks the slice to
//! write a session to a file, and the event it publishes once the file is on
//! disk. The chat-input slice's `/export` slash command depends on this crate
//! to construct the command; the export slice's actor consumes it over the
//! `jinn.export` trouper topic.
//!
//! Only the types named across that boundary live here. The export format
//! trait, the document model, path resolution, and the renderers are all
//! internal to the `jinn-export` slice.

pub use jinn_core_types::SessionId;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Ask the export slice to write a session to a file.
///
/// Published by the `/export` slash command. The `path` is what the user
/// typed, resolved against the session's working directory by the export
/// actor: an empty path selects a default filename, an absolute path is used
/// as given, and a relative path is joined onto the session's cwd. The
/// format is chosen from the resolved path's extension.
///
/// The actor snapshots the session under the state read lock, releases it,
/// then renders and writes on a blocking worker. Success pushes a transient
/// chat notice naming the written file and publishes [`ExportCompleted`];
/// failure pushes an `Error` chat entry on the session.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Ask the export slice to write a session to a file.")]
pub struct ExportSessionToFile {
    /// The session to export.
    pub session_id: SessionId,
    /// Destination path, as typed. Empty means "pick a default filename".
    pub path: PathBuf,
}

impl jinn_slices::BusMessage for ExportSessionToFile {}

/// The export slice finished writing a session to disk.
///
/// Broadcast after a successful write so interested parties (a status hint,
/// a test) can react without polling the filesystem. Failures do not publish
/// this; they surface as an `Error` chat entry on the target session instead,
/// which the user can see without a subscriber.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "The export slice finished writing a session to disk.")]
pub struct ExportCompleted {
    /// The session that was exported.
    pub session_id: SessionId,
    /// The path the document was written to.
    pub path: PathBuf,
}

impl jinn_slices::BusMessage for ExportCompleted {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    #[test]
    fn roundtrips_through_json() {
        // Given a command naming a session and a destination path.
        let session_id = SessionId::new();
        let cmd = ExportSessionToFile {
            session_id: session_id.clone(),
            path: PathBuf::from("/tmp/out.html"),
        };

        // When serializing and deserializing.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: ExportSessionToFile = serde_json::from_str(&json).expect("deserialize");

        // Then the roundtrip preserves both fields.
        assert_eq!(back.session_id, session_id);
        assert_eq!(back.path, PathBuf::from("/tmp/out.html"));
    }

    #[rstest::rstest]
    #[test]
    fn deserializes_from_json_payload() {
        // Given a JSON payload shaped like a bridge relay's deserialized body.
        let payload = serde_json::json!({
            "session_id": "01933dc5-2b14-7e21-8f52-3d1d8f4e7f9a",
            "path": "notes/session.md"
        });

        // When deserializing into the command.
        let msg: ExportSessionToFile = serde_json::from_value(payload).unwrap();

        // Then the fields carry through.
        assert_eq!(
            msg.session_id.to_string(),
            "01933dc5-2b14-7e21-8f52-3d1d8f4e7f9a"
        );
        assert_eq!(msg.path, PathBuf::from("notes/session.md"));
    }

    #[rstest::rstest]
    #[test]
    fn command_schema_def_carries_name_kind_and_fields() {
        // Given the command's schema definition.
        let schema = <ExportSessionToFile as trouper::schema::Schema>::schema_def();

        // When inspecting its name, kind, and fields.
        // Then it is a command named ExportSessionToFile with both fields.
        assert_eq!(schema.name, "ExportSessionToFile");
        assert!(matches!(schema.kind, trouper::schema::SchemaKind::Command));
        let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["session_id", "path"]);
    }

    #[rstest::rstest]
    #[test]
    fn event_schema_def_is_an_event() {
        // Given the event's schema definition.
        let schema = <ExportCompleted as trouper::schema::Schema>::schema_def();

        // When inspecting its kind.
        // Then it is an event, not a command.
        assert_eq!(schema.name, "ExportCompleted");
        assert!(matches!(schema.kind, trouper::schema::SchemaKind::Event));
    }
}
