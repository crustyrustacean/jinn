use std::collections::HashMap;

use crate::definition::{ModelHint, StepDef};
use crate::guard::GuardExpr;
use crate::{StepStatus, WorkflowDef, WorkflowState};

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

#[rstest::rstest]
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

#[rstest::rstest]
fn start_activates_first_step() {
    // Given a workflow state with 3 steps.
    let mut state = WorkflowState::new(make_workflow(3));

    // When starting the workflow.
    state.start().unwrap();

    // Then the first step is active.
    assert_eq!(state.active_step.as_deref(), Some("step-0"));
    assert_eq!(state.steps["step-0"].status.clone(), StepStatus::Active);
}

#[rstest::rstest]
fn start_fails_with_no_steps() {
    // Given a workflow with no steps.
    let mut state = WorkflowState::new(make_workflow(0));

    // When starting.
    let result = state.start();

    // Then it returns an error.
    assert!(result.is_err());
}

// ---- advance ----

#[rstest::rstest]
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

#[rstest::rstest]
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

#[rstest::rstest]
fn jump_activates_target() {
    // Given a workflow where steps 0 and 1 are completed.
    let mut state = WorkflowState::new(make_workflow(3));
    state.start().unwrap();
    state.complete_step("step-0", HashMap::new()).unwrap();
    state.finalize_step("step-0").unwrap();
    state.advance();
    state.complete_step("step-1", HashMap::new()).unwrap();
    state.finalize_step("step-1").unwrap();

    // When jumping back to step 0.
    let _stale_steps = state.jump_to("step-0").unwrap();

    // Then step 0 is active.
    assert_eq!(state.steps["step-0"].status.clone(), StepStatus::Active);
}

#[rstest::rstest]
fn jump_marks_downstream_stale() {
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
    assert_eq!(state.steps["step-1"].status.clone(), StepStatus::Stale);
    assert_eq!(state.steps["step-2"].status.clone(), StepStatus::Stale);
}

#[rstest::rstest]
fn jump_to_with_invalid_step_returns_error() {
    let mut state = WorkflowState::new(make_workflow(2));
    state.start().unwrap();

    let result = state.jump_to("nonexistent");
    assert!(result.is_err());
}

// ---- complete_step ----

#[rstest::rstest]
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

#[rstest::rstest]
fn complete_step_returns_error_for_unknown_step() {
    let mut state = WorkflowState::new(make_workflow(1));
    let result = state.complete_step("nope", HashMap::new());
    assert!(result.is_err());
}

// ---- finalize_step ----

#[rstest::rstest]
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

#[rstest::rstest]
fn finalize_step_returns_error_for_unknown_step() {
    let mut state = WorkflowState::new(make_workflow(1));
    let result = state.finalize_step("nope");
    assert!(result.is_err());
}

// ---- downstream_steps ----

#[rstest::rstest]
fn downstream_steps_returns_correct_ids() {
    let state = WorkflowState::new(make_workflow(4));

    let ds = state.downstream_steps("step-1");
    assert_eq!(ds, vec!["step-2", "step-3"]);

    let ds_last = state.downstream_steps("step-3");
    assert!(ds_last.is_empty());
}

// ---- full lifecycle ----

#[rstest::rstest]
fn complete_then_finalize_advances() {
    // Given a started workflow with 2 steps.
    let mut state = WorkflowState::new(make_workflow(2));
    state.start().unwrap();

    // When completing, finalizing, and advancing step 0.
    state.complete_step("step-0", HashMap::new()).unwrap();
    assert_eq!(state.steps["step-0"].status, StepStatus::AwaitingInput);
    state.finalize_step("step-0").unwrap();
    let next = state.advance();

    // Then the next step is returned.
    assert_eq!(next.as_deref(), Some("step-1"));
}

// ---- jump-back lifecycle ----

// ---- step_order ----

#[rstest::rstest]
fn step_order_returns_definition_order() {
    let state = WorkflowState::new(make_workflow(3));
    assert_eq!(state.step_order(), vec!["step-0", "step-1", "step-2"]);
}
