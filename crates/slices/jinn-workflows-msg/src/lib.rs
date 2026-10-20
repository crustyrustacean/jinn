//! Workflows crossing contracts — the [`Workflow`] payload, the picker's
//! state and identity, and the [`WorkflowsLoaded`] event the session-init
//! discovery worker broadcasts.
//!
//! The scanning actor lives in the session-init slice; this crate is the
//! pure contract: data and pure functions only.

mod picker_scope;
mod picker_state;
mod workflow;

pub use picker_scope::RESULTS_VIEWPORT_FALLBACK;
pub use picker_scope::workflows_picker_scope;
pub use picker_scope::workflows_picker_slot;
pub use picker_state::WorkflowEntry;
pub use picker_state::WorkflowPickerState;
pub use picker_state::build_workflow_entries;
pub use picker_state::workflow_row;
pub use picker_state::workflow_search_text;
pub use workflow::Workflow;

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Command to trigger a workflows rescan for a specific session.
///
/// The picker's rescan row publishes this; the session-init supervisor
/// forwards it to the discovery worker as its worker-internal
/// `RescanWorkflows` command — the same Scan→Rescan shape `ScanSkills` →
/// `RescanSkills` already uses. (The trigger cannot share the command's
/// name: trouper's schema registry keys by schema name, and one name owns
/// one Rust type.) Workflows are launch-global (the config dir), so `cwd`
/// rides along for shape parity with the other triggers and is not
/// scanned.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Trigger a workflows rescan for a session.")]
pub struct ScanWorkflows {
    /// The session whose scan this is.
    pub session_id: SessionId,
    /// The working directory driving the scan (unused by the workflows
    /// resource; carried for shape parity with the other triggers).
    #[serde(default)]
    pub cwd: PathBuf,
}

impl jinn_slices::BusMessage for ScanWorkflows {}

/// Emitted when a session's workflow scan has completed.
///
/// On success, `workflows` contains every admitted workflow and `error` is
/// `None`. On failure, `workflows` is empty and `error` carries a
/// description. A scan that found nothing valid also publishes with an
/// empty list — an empty result is a result, not a failure.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's workflow scan has completed.")]
pub struct WorkflowsLoaded {
    /// The session whose scan this is.
    pub session_id: SessionId,
    /// The discovered workflows (admitted ones only).
    pub workflows: Vec<Workflow>,
    /// Error message if scanning failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl jinn_slices::BusMessage for WorkflowsLoaded {}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unreachable, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn workflows_loaded_roundtrips_all_fields() {
        // Given a loaded event carrying one workflow and no error. The
        // workflow's directory is built from a temp root, so it is an
        // absolute path on every platform.
        let root = jinn_testutil::tempdir().expect("temp dir");
        let event = WorkflowsLoaded {
            session_id: SessionId::new(),
            workflows: vec![Workflow {
                name: "research-report".to_owned(),
                description: "Research a topic and deliver a cited brief.".to_owned(),
                dir: root.path().join("workflows").join("research-report"),
            }],
            error: None,
        };

        // When serializing then deserializing.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: WorkflowsLoaded = serde_json::from_str(&json).expect("deserialize");

        // Then the fields are preserved.
        assert_eq!(back.session_id, event.session_id);
        assert_eq!(back.workflows, event.workflows);
        assert_eq!(back.error, None);
    }

    #[rstest::rstest]
    fn workflows_loaded_omits_error_when_none() {
        // Given a loaded event with no error.
        let event = WorkflowsLoaded {
            session_id: SessionId::new(),
            workflows: vec![],
            error: None,
        };

        // When serializing.
        let json = serde_json::to_string(&event).expect("serialize");

        // Then the `error` key is absent from the wire.
        assert!(!json.contains("error"), "got: {json}");
    }

    #[rstest::rstest]
    fn workflows_loaded_schema_id_is_stable() {
        // Given the crossing contract.
        // When deriving its trouper schema id.
        // Then the id equals the type name (the compatibility surface).
        assert_eq!(
            <WorkflowsLoaded as trouper::schema::Schema>::schema_id().name(),
            "WorkflowsLoaded"
        );
    }

    #[rstest::rstest]
    fn scan_workflows_schema_id_is_stable() {
        // Given the crossing contract.
        // When deriving its trouper schema id.
        // Then the id equals the type name (the compatibility surface).
        assert_eq!(
            <ScanWorkflows as trouper::schema::Schema>::schema_id().name(),
            "ScanWorkflows"
        );
    }

    #[rstest::rstest]
    fn the_picker_slot_and_scope_are_stable() {
        // Given the picker's identity accessors.
        // When reading the slot and scope twice.
        // Then both are stable values.
        assert_eq!(workflows_picker_slot(), workflows_picker_slot());
        assert_eq!(workflows_picker_scope(), workflows_picker_scope());
    }
}
