//! Persistence tests for the lifecycle source marker: the field survives
//! the snapshot roundtrip and the store save/load, reads as absent on
//! pre-marker metadata, and rides a fork with the name and args it
//! qualifies.

#![allow(clippy::expect_used, reason = "test code")]

use crate::ChatSessionState;
use crate::snapshot::SessionSnapshotMetadata;

/// Metadata from a real session capture, so the test exercises the same
/// construction production does.
fn metadata() -> SessionSnapshotMetadata {
    ChatSessionState::new().capture_snapshot().metadata
}

#[rstest::rstest]
fn a_lifecycle_source_marker_survives_the_metadata_roundtrip() {
    // Given metadata stamped with a lifecycle source.
    let mut marked = metadata();
    marked.lifecycle_source = Some("workflows/autocoder/lifecycle.toml".to_owned());

    // When serializing and deserializing.
    let json = serde_json::to_string(&marked).expect("serialize");
    let round: SessionSnapshotMetadata = serde_json::from_str(&json).expect("deserialize");

    // Then the marker survives, so a restarted session's teardown still
    // re-reads the workflow's own file.
    assert_eq!(
        round.lifecycle_source,
        Some("workflows/autocoder/lifecycle.toml".to_owned())
    );
}

#[rstest::rstest]
fn metadata_without_a_marker_roundtrips_as_none() {
    // Given metadata with no marker (every jinn.toml lifecycle session).
    let marked = metadata();
    assert_eq!(marked.lifecycle_source, None);

    // When serializing.
    let json = serde_json::to_string(&marked).expect("serialize");

    // Then the key is omitted entirely, so a blob is byte-identical to one
    // written before the marker existed.
    assert!(!json.contains("lifecycle_source"));
    let round: SessionSnapshotMetadata = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round.lifecycle_source, None);
}

#[rstest::rstest]
fn pre_marker_metadata_loads_with_no_marker() {
    // Given metadata JSON written before the marker existed.
    let mut value = serde_json::to_value(metadata()).expect("serialize");
    let value = value.as_object_mut().expect("object").clone();
    debug_assert!(!value.contains_key("lifecycle_source"));

    // When deserializing.
    let loaded: SessionSnapshotMetadata =
        serde_json::from_value(serde_json::Value::Object(value)).expect("pre-marker metadata");

    // Then the load succeeds with no marker, rather than failing.
    assert_eq!(loaded.lifecycle_source, None);
}

#[rstest::rstest]
fn the_marker_rides_the_session_core_roundtrip() {
    // Given a live session stamped with a lifecycle source.
    let mut session = ChatSessionState::new();
    session.set_lifecycle_name(Some("stage".to_owned()));
    session.set_lifecycle_source(Some("workflows/autocoder/lifecycle.toml".to_owned()));

    // When capturing a snapshot and rebuilding core from it.
    let snapshot = session.capture_snapshot();
    let restored = crate::SessionCore::from(snapshot.metadata);

    // Then the restored core still knows its source file — and its name,
    // the entry the file is read for.
    assert_eq!(
        restored.lifecycle.lifecycle_source,
        Some("workflows/autocoder/lifecycle.toml".to_owned())
    );
    assert_eq!(restored.lifecycle.lifecycle_name, Some("stage".to_owned()));
}

#[rstest::rstest]
fn the_marker_crosses_a_fork_with_the_name_it_qualifies() {
    // Given a live session stamped with a workflow-local lifecycle.
    let mut source = ChatSessionState::new();
    source.push_entry(jinn_core_types::ChatEntry::user("history"));
    source.set_lifecycle_name(Some("stage".to_owned()));
    source.set_lifecycle_source(Some("workflows/autocoder/lifecycle.toml".to_owned()));
    let snapshot = source.capture_snapshot();

    // When forking it.
    let forked = snapshot.forked_from(jinn_core_types::SessionId::new(), 0);

    // Then the fork carries the same coordinates: it inherited the
    // lifecycle name and args, so it must resolve them from the same file
    // — a fork that dropped the marker would look the name up in
    // jinn.toml and skip its teardown silently.
    assert_eq!(
        forked.metadata.lifecycle_source,
        Some("workflows/autocoder/lifecycle.toml".to_owned())
    );
    assert_eq!(forked.metadata.lifecycle_name, Some("stage".to_owned()));
}
