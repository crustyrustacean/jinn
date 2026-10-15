//! Persistence tests for the workflow-source marker: the field survives the
//! snapshot roundtrip and reads as absent on pre-marker metadata.

#![allow(clippy::expect_used, reason = "test code")]

use crate::ChatSessionState;
use crate::snapshot::SessionSnapshotMetadata;

/// Metadata from a real session capture, so the test exercises the same
/// construction production does.
fn metadata() -> SessionSnapshotMetadata {
    ChatSessionState::new().capture_snapshot().metadata
}

#[rstest::rstest]
fn a_workflow_source_marker_survives_the_metadata_roundtrip() {
    // Given metadata stamped with a workflow source.
    let mut marked = metadata();
    marked.workflow_source = Some(jinn_core_types::AttendantSourcePath::new(
        "workflows/autocoder/attendants/planner.toml",
    ));

    // When serializing and deserializing.
    let json = serde_json::to_string(&marked).expect("serialize");
    let round: SessionSnapshotMetadata = serde_json::from_str(&json).expect("deserialize");

    // Then the marker survives, so a restarted workflow attendant still
    // refuses saves and still knows its source file.
    assert_eq!(
        round.workflow_source,
        Some(jinn_core_types::AttendantSourcePath::new(
            "workflows/autocoder/attendants/planner.toml"
        ))
    );
}

#[rstest::rstest]
fn metadata_without_a_marker_roundtrips_as_none() {
    // Given metadata with no marker (every personal session).
    let marked = metadata();
    assert_eq!(marked.workflow_source, None);

    // When serializing and deserializing.
    let json = serde_json::to_string(&marked).expect("serialize");

    // Then the key is omitted entirely, so a blob is byte-identical to one
    // written before the marker existed.
    assert!(!json.contains("workflow_source"));
    let round: SessionSnapshotMetadata = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round.workflow_source, None);
}

#[rstest::rstest]
fn pre_marker_metadata_loads_with_no_marker() {
    // Given metadata JSON written before the marker existed.
    let mut value = serde_json::to_value(metadata()).expect("serialize");
    let value = value.as_object_mut().expect("object").clone();
    debug_assert!(!value.contains_key("workflow_source"));

    // When deserializing.
    let loaded: SessionSnapshotMetadata =
        serde_json::from_value(serde_json::Value::Object(value)).expect("pre-marker metadata");

    // Then the load succeeds with no marker, rather than failing.
    assert_eq!(loaded.workflow_source, None);
}

#[rstest::rstest]
fn the_marker_rides_the_session_core_roundtrip() {
    // Given a live attendant stamped with a workflow source.
    let parent = ChatSessionState::new();
    let mut attendant = ChatSessionState::new_attendant(&parent, true);
    attendant.set_workflow_source(jinn_core_types::AttendantSourcePath::new(
        "attendants/qa.toml",
    ));

    // When capturing a snapshot and rebuilding core from it.
    let snapshot = attendant.capture_snapshot();
    let restored = crate::SessionCore::from(snapshot.metadata);

    // Then the restored core still knows its source file.
    assert_eq!(
        restored.attendant.workflow_source,
        Some(jinn_core_types::AttendantSourcePath::new(
            "attendants/qa.toml"
        ))
    );
}
