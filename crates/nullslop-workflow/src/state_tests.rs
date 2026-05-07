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
