//! Workflow state machine — step statuses, transitions, and invalidation.
//!
//! Manages the runtime state of an active workflow: which step is current, which
//! are complete, and which are stale due to jump-backs. Content-hash-based
//! invalidation prevents wasteful re-execution when downstream outputs are unchanged.

use std::collections::HashMap;

use error_stack::Report;
use serde::{Deserialize, Serialize};

use crate::definition::{StepDef, WorkflowDef};
use crate::error::{WorkflowError, WorkflowErrorKind};
use crate::hash::file_content_hash;

/// The current status of a workflow step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepStatus {
    /// Not yet started.
    Pending,
    /// Currently executing.
    Active,
    /// Completed successfully (guards passed).
    Completed,
    /// Awaiting user approval (checkpoint) or user input.
    AwaitingInput,
    /// Invalidated by a jump-back. Must be re-done.
    Stale,
}

/// Runtime state for a single step.
///
/// Tracks the step's current status, output hashes captured at completion time,
/// and resolved output values for context assembly in downstream steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepState {
    /// The step definition.
    pub def: StepDef,
    /// Current status.
    pub status: StepStatus,
    /// Content hashes of file outputs captured at completion time.
    /// Maps resolved file path → SHA-256 hash.
    #[serde(default)]
    pub output_hashes: HashMap<String, String>,
    /// Resolved output values captured at completion time.
    /// Maps output label → resolved value.
    #[serde(default)]
    pub resolved_outputs: HashMap<String, String>,
}

/// Runtime state of an active workflow.
///
/// Created from a [`WorkflowDef`] and evolved through the workflow lifecycle.
/// Supports start, advance, `complete_step`, and `jump_to` transitions.
/// Serializes to JSON for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    /// The workflow definition.
    pub definition: WorkflowDef,
    /// State for each step, indexed by step ID.
    pub steps: HashMap<String, StepState>,
    /// The current active step ID.
    pub active_step: Option<String>,
    /// Resolved global variables.
    pub globals: HashMap<String, String>,
}

impl WorkflowState {
    /// Creates a new workflow state from a definition.
    ///
    /// All steps start as [`StepStatus::Pending`] with no active step.
    /// Call [`start`](Self::start) to activate the first step.
    pub fn new(definition: WorkflowDef) -> Self {
        let globals = definition.globals.clone();
        let steps = definition
            .steps
            .iter()
            .map(|step_def| {
                (
                    step_def.id.clone(),
                    StepState {
                        def: step_def.clone(),
                        status: StepStatus::Pending,
                        output_hashes: HashMap::new(),
                        resolved_outputs: HashMap::new(),
                    },
                )
            })
            .collect();

        Self {
            definition,
            steps,
            active_step: None,
            globals,
        }
    }

    /// Starts the workflow by activating the first step.
    ///
    /// # Errors
    ///
    /// Returns an error if the workflow has no steps.
    pub fn start(&mut self) -> Result<(), Report<WorkflowError>> {
        let first_id = self
            .definition
            .steps
            .first()
            .map(|s| s.id.clone())
            .ok_or_else(|| {
                Report::new(WorkflowError::new(WorkflowErrorKind::EmptyWorkflow))
                    .attach("cannot start a workflow with no steps")
            })?;

        if let Some(step) = self.steps.get_mut(&first_id) {
            step.status = StepStatus::Active;
        }
        self.active_step = Some(first_id);
        Ok(())
    }

    /// Advances to the next step.
    ///
    /// Returns the step ID of the next step, or `None` if the workflow is complete.
    pub fn advance(&mut self) -> Option<String> {
        let current_id = self.active_step.as_ref()?;
        let order = self.step_order();
        let current_idx = order.iter().position(|id| id == current_id)?;
        let next_idx = current_idx + 1;

        let next_id = order.get(next_idx)?.clone();

        // Mark current as completed (should already be, but ensure).
        if let Some(step) = self.steps.get_mut(current_id)
            && step.status == StepStatus::Active
        {
            step.status = StepStatus::Completed;
        }

        // Activate next step.
        if let Some(step) = self.steps.get_mut(&next_id) {
            step.status = StepStatus::Active;
        }
        self.active_step = Some(next_id.clone());

        Some(next_id)
    }

    /// Jumps to a specific step by ID.
    ///
    /// Marks all downstream steps as [`StepStatus::Stale`]. Returns the list of
    /// step IDs that were marked stale.
    ///
    /// # Errors
    ///
    /// Returns an error if the step ID doesn't exist.
    pub fn jump_to(&mut self, step_id: &str) -> Result<Vec<String>, Report<WorkflowError>> {
        if !self.steps.contains_key(step_id) {
            return Err(
                Report::new(WorkflowError::new(WorkflowErrorKind::StepNotFound(
                    step_id.to_owned(),
                )))
                .attach("cannot jump to unknown step"),
            );
        }

        // Mark current as completed if active.
        if let Some(current) = self.active_step.as_deref()
            && let Some(step) = self.steps.get_mut(current)
            && step.status == StepStatus::Active
        {
            step.status = StepStatus::Completed;
        }

        // Find downstream steps and mark them stale.
        let downstream = self.downstream_steps(step_id);
        for ds_id in &downstream {
            if let Some(step) = self.steps.get_mut(ds_id) {
                step.status = StepStatus::Stale;
            }
        }

        // Activate target step.
        if let Some(step) = self.steps.get_mut(step_id) {
            step.status = StepStatus::Active;
        }
        self.active_step = Some(step_id.to_owned());

        Ok(downstream)
    }

    /// Records that a step completed, capturing output hashes.
    ///
    /// Sets the step status to [`StepStatus::AwaitingInput`] (pending user approval)
    /// and stores content hashes for file outputs and resolved output values.
    /// The step is finalized as [`StepStatus::Completed`] by [`finalize_step`](Self::finalize_step)
    /// when the user approves and advances.
    ///
    /// # Errors
    ///
    /// Returns an error if the step ID doesn't exist.
    pub fn complete_step(
        &mut self,
        step_id: &str,
        resolved_outputs: HashMap<String, String>,
    ) -> Result<(), Report<WorkflowError>> {
        let step = self.steps.get_mut(step_id).ok_or_else(|| {
            Report::new(WorkflowError::new(WorkflowErrorKind::StepNotFound(
                step_id.to_owned(),
            )))
        })?;

        step.status = StepStatus::AwaitingInput;
        step.resolved_outputs = resolved_outputs;

        // Hash file outputs.
        step.output_hashes.clear();
        for output in &step.def.outputs {
            if let crate::definition::StepOutputDef::File { path, .. } = output {
                let resolved_path = crate::template::resolve_template(path, &self.globals);
                if let Some(hash) = file_content_hash(std::path::Path::new(&resolved_path)) {
                    step.output_hashes.insert(resolved_path, hash);
                }
            }
        }

        Ok(())
    }

    /// Marks a completed step as finalized.
    ///
    /// Called by the advance handler after user approval. Transitions the step
    /// from [`StepStatus::AwaitingInput`] to [`StepStatus::Completed`].
    ///
    /// # Errors
    ///
    /// Returns an error if the step ID doesn't exist.
    pub fn finalize_step(&mut self, step_id: &str) -> Result<(), Report<WorkflowError>> {
        let step = self.steps.get_mut(step_id).ok_or_else(|| {
            Report::new(WorkflowError::new(WorkflowErrorKind::StepNotFound(
                step_id.to_owned(),
            )))
        })?;
        step.status = StepStatus::Completed;
        Ok(())
    }

    /// Checks if a stale downstream step is still valid by comparing content hashes.
    ///
    /// Returns `true` if all of the step's file outputs have unchanged hashes,
    /// meaning the step does not need to be re-executed.
    pub fn is_step_output_valid(&self, step_id: &str) -> bool {
        let Some(step) = self.steps.get(step_id) else {
            return false;
        };

        // If the step has no file outputs, it's valid by default.
        if step.output_hashes.is_empty() {
            return true;
        }

        // Check each stored hash against the current file content.
        for (path, stored_hash) in &step.output_hashes {
            match file_content_hash(std::path::Path::new(path)) {
                Some(current_hash) if &current_hash == stored_hash => {}
                _ => return false,
            }
        }

        true
    }

    /// Returns the ordered list of step IDs (in definition order).
    pub fn step_order(&self) -> Vec<String> {
        self.definition.steps.iter().map(|s| s.id.clone()).collect()
    }

    /// Returns the step IDs that are downstream from the given step.
    ///
    /// Downstream means all steps that appear after the given step in
    /// definition order.
    pub fn downstream_steps(&self, step_id: &str) -> Vec<String> {
        let order = self.step_order();
        let Some(idx) = order.iter().position(|id| id == step_id) else {
            return vec![];
        };
        order
            .get((idx + 1)..)
            .map(<[String]>::to_vec)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{ModelHint, StepDef};
    use crate::guard::GuardExpr;

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

    // ---- new ----

    #[test]
    fn new_creates_all_steps_as_pending() {
        // Given a workflow definition with 3 steps.
        let def = make_workflow(3);

        // When creating workflow state.
        let state = WorkflowState::new(def);

        // Then all steps are pending and no step is active.
        assert!(state.active_step.is_none());
        for step in state.steps.values() {
            assert_eq!(step.status, StepStatus::Pending);
        }
    }

    // ---- start ----

    #[test]
    fn start_activates_first_step() {
        // Given a workflow state with 3 steps.
        let mut state = WorkflowState::new(make_workflow(3));

        // When starting the workflow.
        state.start().unwrap();

        // Then the first step is active.
        assert_eq!(state.active_step.as_deref(), Some("step-0"));
        assert_eq!(state.steps["step-0"].status.clone(), StepStatus::Active);
    }

    #[test]
    fn start_fails_with_no_steps() {
        // Given a workflow with no steps.
        let mut state = WorkflowState::new(make_workflow(0));

        // When starting.
        let result = state.start();

        // Then it returns an error.
        assert!(result.is_err());
    }

    // ---- advance ----

    #[test]
    fn advance_moves_to_next_step() {
        // Given a started workflow.
        let mut state = WorkflowState::new(make_workflow(3));
        state.start().unwrap();

        // When advancing.
        let next = state.advance();

        // Then the next step is returned and activated.
        assert_eq!(next.as_deref(), Some("step-1"));
        assert_eq!(state.active_step.as_deref(), Some("step-1"));
        assert_eq!(state.steps["step-0"].status.clone(), StepStatus::Completed);
    }

    #[test]
    fn advance_on_last_step_returns_none() {
        // Given a workflow on its last step.
        let mut state = WorkflowState::new(make_workflow(1));
        state.start().unwrap();

        // When advancing past the last step.
        let next = state.advance();

        // Then there is no next step.
        assert!(next.is_none());
    }

    // ---- jump_to ----

    #[test]
    fn jump_to_activates_target_and_marks_downstream_stale() {
        // Given a workflow where steps 0 and 1 are completed.
        let mut state = WorkflowState::new(make_workflow(3));
        state.start().unwrap();
        state.complete_step("step-0", HashMap::new()).unwrap();
        state.finalize_step("step-0").unwrap();
        state.advance();
        state.complete_step("step-1", HashMap::new()).unwrap();
        state.finalize_step("step-1").unwrap();

        // When jumping back to step 0.
        let stale_steps = state.jump_to("step-0").unwrap();

        // Then downstream steps are stale.
        assert_eq!(stale_steps, vec!["step-1", "step-2"]);
        assert_eq!(state.steps["step-0"].status.clone(), StepStatus::Active);
        assert_eq!(state.steps["step-1"].status.clone(), StepStatus::Stale);
        assert_eq!(state.steps["step-2"].status.clone(), StepStatus::Stale);
    }

    #[test]
    fn jump_to_with_invalid_step_returns_error() {
        let mut state = WorkflowState::new(make_workflow(2));
        state.start().unwrap();

        let result = state.jump_to("nonexistent");
        assert!(result.is_err());
    }

    // ---- complete_step ----

    #[test]
    fn complete_step_sets_awaiting_input_and_records_outputs() {
        // Given an active step.
        let mut state = WorkflowState::new(make_workflow(1));
        state.start().unwrap();

        // When completing the step with outputs.
        let outputs = HashMap::from([("dir".to_owned(), "/tmp/test".to_owned())]);
        state.complete_step("step-0", outputs).unwrap();

        // Then the step is awaiting input with outputs recorded.
        let step = &state.steps["step-0"];
        assert_eq!(step.status, StepStatus::AwaitingInput);
        assert_eq!(
            step.resolved_outputs.get("dir"),
            Some(&"/tmp/test".to_owned())
        );
    }

    #[test]
    fn complete_step_returns_error_for_unknown_step() {
        let mut state = WorkflowState::new(make_workflow(1));
        let result = state.complete_step("nope", HashMap::new());
        assert!(result.is_err());
    }

    // ---- finalize_step ----

    #[test]
    fn finalize_step_sets_completed() {
        // Given a step in AwaitingInput status.
        let mut state = WorkflowState::new(make_workflow(1));
        state.start().unwrap();
        state.complete_step("step-0", HashMap::new()).unwrap();
        assert_eq!(state.steps["step-0"].status, StepStatus::AwaitingInput);

        // When finalizing the step.
        state.finalize_step("step-0").unwrap();

        // Then the step is completed.
        assert_eq!(state.steps["step-0"].status, StepStatus::Completed);
    }

    #[test]
    fn finalize_step_returns_error_for_unknown_step() {
        let mut state = WorkflowState::new(make_workflow(1));
        let result = state.finalize_step("nope");
        assert!(result.is_err());
    }

    // ---- downstream_steps ----

    #[test]
    fn downstream_steps_returns_correct_ids() {
        let state = WorkflowState::new(make_workflow(4));

        let ds = state.downstream_steps("step-1");
        assert_eq!(ds, vec!["step-2", "step-3"]);

        let ds_last = state.downstream_steps("step-3");
        assert!(ds_last.is_empty());
    }

    // ---- full lifecycle ----

    #[test]
    fn full_lifecycle_start_complete_finalize_advance() {
        // Given a workflow with 2 steps.
        let mut state = WorkflowState::new(make_workflow(2));

        // When running through the full lifecycle.
        state.start().unwrap();
        assert_eq!(state.active_step.as_deref(), Some("step-0"));

        state.complete_step("step-0", HashMap::new()).unwrap();
        assert_eq!(state.steps["step-0"].status, StepStatus::AwaitingInput);
        state.finalize_step("step-0").unwrap();
        let next = state.advance();
        assert_eq!(next.as_deref(), Some("step-1"));

        state.complete_step("step-1", HashMap::new()).unwrap();
        assert_eq!(state.steps["step-1"].status, StepStatus::AwaitingInput);
        state.finalize_step("step-1").unwrap();
        let done = state.advance();

        // Then the workflow is complete (no more steps).
        assert!(done.is_none());
    }

    // ---- jump-back lifecycle ----

    #[test]
    fn jump_back_marks_downstream_as_stale() {
        let mut state = WorkflowState::new(make_workflow(3));
        state.start().unwrap();
        state.complete_step("step-0", HashMap::new()).unwrap();
        state.finalize_step("step-0").unwrap();
        state.advance();
        state.complete_step("step-1", HashMap::new()).unwrap();
        state.finalize_step("step-1").unwrap();

        // Jump back to step 0.
        let stale_steps = state.jump_to("step-0").unwrap();

        assert_eq!(stale_steps, vec!["step-1", "step-2"]);
        assert_eq!(state.active_step.as_deref(), Some("step-0"));
        assert_eq!(state.steps["step-1"].status.clone(), StepStatus::Stale);
    }

    // ---- step_order ----

    #[test]
    fn step_order_returns_definition_order() {
        let state = WorkflowState::new(make_workflow(3));
        assert_eq!(state.step_order(), vec!["step-0", "step-1", "step-2"]);
    }

    // ---- WorkflowState roundtrip ----

    #[test]
    fn workflow_state_roundtrips_through_serde() {
        let mut state = WorkflowState::new(make_workflow(2));
        state.start().unwrap();
        state.complete_step("step-0", HashMap::new()).unwrap();
        state.finalize_step("step-0").unwrap();

        let json = serde_json::to_string(&state).unwrap();
        let back: WorkflowState = serde_json::from_str(&json).unwrap();

        assert_eq!(state.active_step, back.active_step);
        assert_eq!(state.steps.len(), back.steps.len());
        assert_eq!(
            state.steps["step-0"].status.clone(),
            back.steps["step-0"].status.clone(),
        );
    }

    // ---- StepStatus roundtrip ----

    #[test]
    fn step_status_roundtrips() {
        let statuses = vec![
            StepStatus::Pending,
            StepStatus::Active,
            StepStatus::Completed,
            StepStatus::AwaitingInput,
            StepStatus::Stale,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let back: StepStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }
}
