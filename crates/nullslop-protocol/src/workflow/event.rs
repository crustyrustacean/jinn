//! Event types for workflow management.

use std::collections::HashMap;

use nullslop_workflow::{GuardExpr, ModelHint, StepOutputDef};
use serde::{Deserialize, Serialize};

use crate::EventMsg;

/// Confirmation that a workflow was loaded and started.
///
/// Emitted by the handler after creating the workflow state machine
/// and activating the first step.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowLoaded {
    /// The name of the loaded workflow.
    pub name: String,
    /// The number of steps in the workflow.
    pub step_count: usize,
}

/// A step has become active.
///
/// Emitted when a step transitions to the
/// [`Active`](nullslop_workflow::StepStatus::Active) status.
/// Contains full step context so the executor actor can operate without
/// `AppState` access.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct StepStarted {
    /// The step ID that became active.
    pub step_id: String,
    /// The step title for display.
    pub step_title: String,

    /// The step's instructions (becomes the system prompt).
    pub instructions: String,
    /// The step's model hint (serialized for future resolution).
    pub model_hint: ModelHint,
    /// The step's model override map from the workflow definition.
    #[serde(default)]
    pub model_overrides: HashMap<String, String>,
    /// Whether this step requires user input before execution.
    pub requires_user_input: bool,
    /// Whether this step requires a checkpoint approval after execution.
    pub checkpoint: bool,
    /// The step's guard expression (for verification after execution).
    #[serde(default)]
    pub guards: GuardExpr,
    /// The step's output descriptors (for hash capture on completion).
    #[serde(default)]
    pub outputs: Vec<StepOutputDef>,
    /// Resolved output values from all completed steps (for context assembly).
    #[serde(default)]
    pub completed_outputs: HashMap<String, HashMap<String, String>>,
    /// Global template variables.
    #[serde(default)]
    pub globals: HashMap<String, String>,
    /// Stored output hashes (for guard evaluation).
    #[serde(default)]
    pub stored_hashes: HashMap<String, String>,
}

/// A step finished successfully.
///
/// Emitted when a step transitions to
/// [`Completed`](nullslop_workflow::StepStatus::Completed).
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct StepCompleted {
    /// The step ID that completed.
    pub step_id: String,
}

/// Steps marked stale by a jump-back.
///
/// Emitted after a [`JumpToStep`](super::JumpToStep) command invalidates
/// downstream steps.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct StepStale {
    /// The step IDs that were marked stale.
    pub step_ids: Vec<String>,
}

/// A step needs user input or approval.
///
/// Emitted when a step has `requires_user_input` or `checkpoint` set
/// and is waiting for the user.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct StepAwaitingInput {
    /// The step ID that is awaiting input.
    pub step_id: String,
}

/// All steps are done.
///
/// Emitted when the last step completes and the workflow has no more
/// steps to execute.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("workflow")]
pub struct WorkflowCompleted;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_loaded_serialization_roundtrip() {
        // Given a WorkflowLoaded event.
        let evt = WorkflowLoaded {
            name: "test-workflow".to_owned(),
            step_count: 5,
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: WorkflowLoaded = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.name, "test-workflow");
        assert_eq!(back.step_count, 5);
    }

    #[test]
    fn workflow_loaded_has_type_name() {
        assert_eq!(WorkflowLoaded::TYPE_NAME, "workflow::WorkflowLoaded");
    }

    #[test]
    fn step_started_serialization_roundtrip() {
        // Given a StepStarted event with all context fields.
        let evt = StepStarted {
            step_id: "step-0".to_owned(),
            step_title: "First Step".to_owned(),
            instructions: "Do the thing".to_owned(),
            model_hint: nullslop_workflow::ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: nullslop_workflow::GuardExpr::None,
            outputs: vec![],
            completed_outputs: HashMap::new(),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: StepStarted = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_id, "step-0");
        assert_eq!(back.step_title, "First Step");
        assert_eq!(back.instructions, "Do the thing");
        assert!(!back.requires_user_input);
    }

    #[test]
    fn step_started_has_type_name() {
        assert_eq!(StepStarted::TYPE_NAME, "workflow::StepStarted");
    }

    #[test]
    fn step_completed_serialization_roundtrip() {
        // Given a StepCompleted event.
        let evt = StepCompleted {
            step_id: "step-0".to_owned(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: StepCompleted = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_id, "step-0");
    }

    #[test]
    fn step_completed_has_type_name() {
        assert_eq!(StepCompleted::TYPE_NAME, "workflow::StepCompleted");
    }

    #[test]
    fn step_stale_serialization_roundtrip() {
        // Given a StepStale event.
        let evt = StepStale {
            step_ids: vec!["step-1".to_owned(), "step-2".to_owned()],
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: StepStale = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_ids, vec!["step-1", "step-2"]);
    }

    #[test]
    fn step_stale_has_type_name() {
        assert_eq!(StepStale::TYPE_NAME, "workflow::StepStale");
    }

    #[test]
    fn step_awaiting_input_serialization_roundtrip() {
        // Given a StepAwaitingInput event.
        let evt = StepAwaitingInput {
            step_id: "step-0".to_owned(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: StepAwaitingInput = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_id, "step-0");
    }

    #[test]
    fn step_awaiting_input_has_type_name() {
        assert_eq!(StepAwaitingInput::TYPE_NAME, "workflow::StepAwaitingInput");
    }

    #[test]
    fn workflow_completed_serialization_roundtrip() {
        // Given a WorkflowCompleted event.
        let evt = WorkflowCompleted;

        // When serialized and deserialized.
        let json = serde_json::to_string(&evt).expect("serialize");
        let back: WorkflowCompleted = serde_json::from_str(&json).expect("deserialize");

        // Then the unit struct roundtrips.
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    #[test]
    fn workflow_completed_has_type_name() {
        assert_eq!(WorkflowCompleted::TYPE_NAME, "workflow::WorkflowCompleted");
    }
}
