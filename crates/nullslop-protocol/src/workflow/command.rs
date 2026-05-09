//! Command types for workflow management.

use std::collections::HashMap;

use nullslop_workflow::WorkflowDef;
use serde::{Deserialize, Serialize};

use crate::CommandMsg;

/// Load a workflow definition and create its state machine.
///
/// The handler creates a [`WorkflowState`](nullslop_workflow::WorkflowState) from the
/// definition, starts it, and emits [`WorkflowLoaded`](super::WorkflowLoaded) and
/// [`StepStarted`](super::StepStarted) events.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct LoadWorkflow {
    /// The workflow definition to load.
    pub definition: WorkflowDef,
}

/// Advance from the current step to the next.
///
/// Finalizes the current step as completed and activates the next one.
/// If there is no next step, emits [`WorkflowCompleted`](super::WorkflowCompleted).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct AdvanceStep;

/// Jump to a specific step, marking downstream steps stale.
///
/// The handler activates the target step and marks all downstream steps as
/// [`Stale`](nullslop_workflow::StepStatus::Stale).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct JumpToStep {
    /// The step ID to jump to.
    pub step_id: String,
}

/// Abort and discard the active workflow.
///
/// Removes the workflow state entirely from
/// `AppState` (in `nullslop-component`).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct AbortWorkflow;

/// Complete a step, recording output hashes and resolved values.
///
/// Called by the executor after guards pass. Calls
/// [`WorkflowState::complete_step()`](nullslop_workflow::WorkflowState::complete_step)
/// to capture file hashes and store resolved output values.
/// The step transitions to `AwaitingInput`, pending user approval.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow")]
pub struct CompleteStep {
    /// The step ID to complete.
    pub step_id: String,
    /// Resolved output values (label → value).
    pub resolved_outputs: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_workflow::{GuardExpr, ModelHint, StepDef, WorkflowDef};

    use super::*;

    /// Creates a minimal workflow definition with the given number of steps.
    fn make_workflow(step_count: usize) -> WorkflowDef {
        let steps: Vec<StepDef> = (0..step_count)
            .map(|i| StepDef {
                id: format!("step-{i}"),
                title: format!("Step {i}"),
                instructions: format!("Instructions for step {i}"),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: false,
                tools: vec![],
                guards: GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            })
            .collect();

        WorkflowDef {
            version: 1,
            name: "test-workflow".to_owned(),
            description: "A test workflow".to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps,
        }
    }

    #[rstest::rstest]
    fn load_workflow_serialization_roundtrip() {
        // Given a LoadWorkflow command.
        let cmd = LoadWorkflow {
            definition: make_workflow(2),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: LoadWorkflow = serde_json::from_str(&json).expect("deserialize");

        // Then the definition is preserved.
        assert_eq!(back.definition.name, "test-workflow");
        assert_eq!(back.definition.steps.len(), 2);
    }

    #[rstest::rstest]
    fn load_workflow_has_command_name() {
        assert_eq!(LoadWorkflow::NAME, "workflow::LoadWorkflow");
    }

    #[rstest::rstest]
    fn advance_step_serialization_roundtrip() {
        // Given an AdvanceStep command.
        let cmd = AdvanceStep;

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: AdvanceStep = serde_json::from_str(&json).expect("deserialize");

        // Then the unit struct roundtrips.
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    #[rstest::rstest]
    fn advance_step_has_command_name() {
        assert_eq!(AdvanceStep::NAME, "workflow::AdvanceStep");
    }

    #[rstest::rstest]
    fn jump_to_step_serialization_roundtrip() {
        // Given a JumpToStep command.
        let cmd = JumpToStep {
            step_id: "create-directory".to_owned(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: JumpToStep = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_id, "create-directory");
    }

    #[rstest::rstest]
    fn jump_to_step_has_command_name() {
        assert_eq!(JumpToStep::NAME, "workflow::JumpToStep");
    }

    #[rstest::rstest]
    fn abort_workflow_serialization_roundtrip() {
        let cmd = AbortWorkflow;
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: AbortWorkflow = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    #[rstest::rstest]
    fn abort_workflow_has_command_name() {
        assert_eq!(AbortWorkflow::NAME, "workflow::AbortWorkflow");
    }

    #[rstest::rstest]
    fn complete_step_serialization_roundtrip() {
        // Given a CompleteStep command.
        let cmd = CompleteStep {
            step_id: "step-0".to_owned(),
            resolved_outputs: HashMap::from([("output_a".to_owned(), "value_a".to_owned())]),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: CompleteStep = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.step_id, "step-0");
        assert_eq!(back.resolved_outputs["output_a"], "value_a");
    }

    #[rstest::rstest]
    fn complete_step_has_command_name() {
        assert_eq!(CompleteStep::NAME, "workflow::CompleteStep");
    }
}
