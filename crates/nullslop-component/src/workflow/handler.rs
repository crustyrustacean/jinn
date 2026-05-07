//! Bus handler for workflow lifecycle commands.
//!
//! Processes [`LoadWorkflow`], [`AdvanceStep`], [`JumpToStep`], [`CompleteStep`],
//! and [`AbortWorkflow`] commands. Mutates [`AppState::workflow`] and emits
//! events for each state transition.
//!
//! Every step pauses after the LLM responds (`AwaitingInput`). Advance only
//! happens when the user explicitly approves via the workflow panel.

use std::collections::HashMap;

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_protocol::CommandAction;
use nullslop_protocol::workflow::{
    AbortWorkflow, AdvanceStep, CompleteStep, JumpToStep, LoadWorkflow, StepAwaitingInput,
    StepCompleted, StepStarted, WorkflowLoaded,
};
use nullslop_services::Services;
use nullslop_workflow::{StepDef, StepStatus, WorkflowState};

use crate::AppState;

define_handler! {
    pub(crate) struct WorkflowHandler;

    commands {
        LoadWorkflow: on_load_workflow,
        AdvanceStep: on_advance_step,
        JumpToStep: on_jump_to_step,
        AbortWorkflow: on_abort_workflow,
        CompleteStep: on_complete_step,
    }

    events {}
}

impl WorkflowHandler {
    /// Loads a workflow definition, starts it, and emits lifecycle events.
    fn on_load_workflow(
        cmd: &LoadWorkflow,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let name = cmd.definition.name.clone();
        let step_count = cmd.definition.steps.len();

        let mut ws = WorkflowState::new(cmd.definition.clone());
        if ws.start().is_err() {
            return CommandAction::Continue;
        }

        ctx.state.active_session_mut().set_workflow(ws);

        ctx.out.submit_event(npr::Event::WorkflowLoaded {
            payload: WorkflowLoaded { name, step_count },
        });

        // Emit StepStarted for the first step.
        if let Some(active_id) = ctx
            .state
            .active_session()
            .workflow()
            .and_then(|w| w.active_step.clone())
        {
            let step_def = ctx
                .state
                .active_session()
                .workflow()
                .and_then(|w| w.steps.get(&active_id).map(|s| &s.def));
            if let Some(def) = step_def {
                let Some(workflow) = ctx.state.active_session().workflow() else {
                    return CommandAction::Continue;
                };
                let started = build_step_started(workflow, &active_id, def);
                ctx.out.submit_event(npr::Event::StepStarted {
                    payload: Box::new(started),
                });
            }
        }

        CommandAction::Continue
    }

    /// Advances from the current step to the next.
    ///
    /// Finalizes the current step as `Completed`, then advances to the next step.
    /// If there is no next step, emits `WorkflowCompleted`.
    fn on_advance_step(
        _cmd: &AdvanceStep,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let Some(ref mut workflow) = ctx.state.active_session_mut().workflow_mut() else {
            return CommandAction::Continue;
        };

        let current_id = workflow.active_step.clone();

        // Finalize current step as Completed.
        if let Some(ref prev_id) = current_id {
            let _ = workflow.finalize_step(prev_id);
            ctx.out.submit_event(npr::Event::StepCompleted {
                payload: StepCompleted {
                    step_id: prev_id.clone(),
                },
            });
        }

        // Advance the state machine.
        let next_id = workflow.advance();

        if let Some(ref nid) = next_id {
            // Emit StepStarted for the new step.
            let step_def = workflow.steps.get(nid).map(|s| &s.def);
            if let Some(def) = step_def {
                let started = build_step_started(workflow, nid, def);
                ctx.out.submit_event(npr::Event::StepStarted {
                    payload: Box::new(started),
                });
            }
        } else {
            // Workflow is complete.
            ctx.out.submit_event(npr::Event::WorkflowCompleted);

            // Post a completion message to the chat log.
            let workflow_name = workflow.definition.name.clone();
            let step_count = workflow.step_order().len();
            let msg =
                format!("✓ Workflow '{workflow_name}' completed — {step_count} steps finished.");
            let entry = npr::ChatEntry::system(msg);
            ctx.state.active_session_mut().push_entry(entry);
        }

        CommandAction::Continue
    }

    /// Jumps to a specific step, marking downstream stale.
    fn on_jump_to_step(
        cmd: &JumpToStep,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let Some(ref mut workflow) = ctx.state.active_session_mut().workflow_mut() else {
            return CommandAction::Continue;
        };

        let Ok(stale_steps) = workflow.jump_to(&cmd.step_id) else {
            return CommandAction::Continue;
        };

        // Emit StepStale if any steps were invalidated.
        if !stale_steps.is_empty() {
            ctx.out.submit_event(npr::Event::StepStale {
                payload: nullslop_protocol::workflow::StepStale {
                    step_ids: stale_steps,
                },
            });
        }

        // Emit StepStarted for the target step.
        let step_def = workflow.steps.get(&cmd.step_id).map(|s| &s.def);
        if let Some(def) = step_def {
            let started = build_step_started(workflow, &cmd.step_id, def);
            ctx.out.submit_event(npr::Event::StepStarted {
                payload: Box::new(started),
            });
        }

        CommandAction::Continue
    }

    /// Aborts and discards the active workflow.
    fn on_abort_workflow(
        _cmd: &AbortWorkflow,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_session_mut().clear_workflow();
        CommandAction::Continue
    }

    /// Completes a step, recording output hashes and resolved values.
    ///
    /// Called by the executor after guards pass. Transitions the step to
    /// `AwaitingInput` and emits a `StepAwaitingInput` event. The user must
    /// explicitly approve before the workflow advances.
    fn on_complete_step(
        cmd: &CompleteStep,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let Some(ref mut workflow) = ctx.state.active_session_mut().workflow_mut() else {
            return CommandAction::Continue;
        };

        let _ = workflow.complete_step(&cmd.step_id, cmd.resolved_outputs.clone());

        ctx.out.submit_event(npr::Event::StepAwaitingInput {
            payload: StepAwaitingInput {
                step_id: cmd.step_id.clone(),
            },
        });

        CommandAction::Continue
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds an enriched [`StepStarted`] event from the current workflow state.
///
/// Populates all context fields the executor actor needs to dispatch LLM calls
/// and evaluate guards without accessing `AppState`.
fn build_step_started(workflow: &WorkflowState, step_id: &str, step_def: &StepDef) -> StepStarted {
    let completed_outputs: HashMap<String, HashMap<String, String>> = workflow
        .steps
        .iter()
        .filter(|(_, s)| matches!(s.status, StepStatus::Completed | StepStatus::AwaitingInput))
        .map(|(id, s)| (id.clone(), s.resolved_outputs.clone()))
        .collect();

    let stored_hashes: HashMap<String, String> = workflow
        .steps
        .iter()
        .flat_map(|(_, s)| s.output_hashes.iter())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    StepStarted {
        step_id: step_id.to_owned(),
        step_title: step_def.title.clone(),
        instructions: step_def.instructions.clone(),
        model_hint: step_def.model_hint.clone(),
        model_overrides: workflow.definition.model_overrides.clone(),
        requires_user_input: step_def.requires_user_input,
        checkpoint: step_def.checkpoint,
        guards: step_def.guards.clone(),
        outputs: step_def.outputs.clone(),
        completed_outputs,
        globals: workflow.globals.clone(),
        stored_hashes,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_component_core::Bus;
    use nullslop_protocol as npr;
    use nullslop_workflow::{GuardExpr, ModelHint, StepDef, StepStatus, WorkflowDef};

    use super::*;
    use crate::AppState;
    use crate::test_utils;
    use crate::workflow::handler::WorkflowHandler;

    /// Creates a minimal workflow definition for testing.
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

    /// Creates a workflow where the first step requires user input.
    fn make_workflow_with_input() -> WorkflowDef {
        let steps = vec![
            StepDef {
                id: "step-0".to_owned(),
                title: "Input Step".to_owned(),
                instructions: "Ask user for something".to_owned(),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: true,
                tools: vec![],
                guards: GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            },
            StepDef {
                id: "step-1".to_owned(),
                title: "Second Step".to_owned(),
                instructions: "Do something else".to_owned(),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: false,
                tools: vec![],
                guards: GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            },
        ];

        WorkflowDef {
            version: 1,
            name: "input-workflow".to_owned(),
            description: "A test workflow with input".to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps,
        }
    }

    /// Sets up a bus with the `WorkflowHandler` registered.
    fn setup_bus() -> Bus<AppState, Services> {
        let mut bus: Bus<AppState, Services> = Bus::new();
        WorkflowHandler.register(&mut bus);
        bus
    }

    /// Helper: process commands and then events, returning the processed events.
    fn process_and_drain_events(
        bus: &mut Bus<AppState, Services>,
        state: &mut AppState,
        services: &Services,
    ) -> Vec<nullslop_component_core::bus::ProcessedEvent> {
        bus.process_commands(state, services);
        bus.process_events(state, services);
        bus.drain_processed_events()
    }

    // --- Test 1: LoadWorkflow creates state and activates first step ---

    #[test]
    fn load_workflow_creates_state_and_activates_first_step() {
        // Given a bus with WorkflowHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();

        // When loading a workflow.
        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(3),
            },
        });
        let mut state = AppState::default();
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then the workflow is active with step-0 as Active.
        assert!(state.active_session().has_workflow());
        assert_eq!(
            state
                .active_session()
                .workflow()
                .unwrap()
                .active_step
                .as_deref(),
            Some("step-0")
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::Active
        );

        // And WorkflowLoaded and StepStarted events were emitted.
        assert_eq!(processed.len(), 2);
        assert!(matches!(
            &processed[0].event,
            npr::Event::WorkflowLoaded { payload } if payload.name == "test-workflow" && payload.step_count == 3
        ));
        assert!(matches!(
            &processed[1].event,
            npr::Event::StepStarted { payload } if payload.step_id == "step-0"
        ));
    }

    // --- Test 2: LoadWorkflow does not emit StepAwaitingInput ---

    #[test]
    fn load_workflow_emits_events_for_first_step() {
        // Given a bus with WorkflowHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();

        // When loading a workflow.
        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(2),
            },
        });
        let mut state = AppState::default();
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then only WorkflowLoaded and StepStarted events were emitted (no StepAwaitingInput on load).
        assert_eq!(processed.len(), 2);
        assert!(matches!(
            &processed[0].event,
            npr::Event::WorkflowLoaded { payload } if payload.name == "test-workflow"
        ));
        assert!(matches!(
            &processed[1].event,
            npr::Event::StepStarted { payload } if payload.step_id == "step-0"
        ));
    }

    // --- Test 3: AdvanceStep finalizes and moves to next step ---

    #[test]
    fn advance_step_finalizes_and_moves_to_next_step() {
        // Given a loaded workflow where step-0 has been completed (AwaitingInput).
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(3),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // Complete step-0 (sets AwaitingInput).
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-0".to_owned(),
                resolved_outputs: HashMap::new(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::AwaitingInput
        );

        // When advancing to the next step.
        bus.submit_command(npr::Command::AdvanceStep);
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then step-0 is finalized as completed and step-1 is active.
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::Completed
        );
        assert_eq!(
            state
                .active_session()
                .workflow()
                .unwrap()
                .active_step
                .as_deref(),
            Some("step-1")
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-1"].status,
            StepStatus::Active
        );

        // And StepCompleted and StepStarted events were emitted.
        assert!(processed.len() >= 2);
        assert!(matches!(
            &processed[0].event,
            npr::Event::StepCompleted { payload } if payload.step_id == "step-0"
        ));
        assert!(matches!(
            &processed[1].event,
            npr::Event::StepStarted { payload } if payload.step_id == "step-1"
        ));
    }

    // --- Test 4: AdvanceStep emits WorkflowCompleted when done ---

    #[test]
    fn advance_step_emits_workflow_completed_when_done() {
        // Given a single-step workflow that has been completed (AwaitingInput).
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(1),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // Complete step-0.
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-0".to_owned(),
                resolved_outputs: HashMap::new(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // When advancing past the last step.
        bus.submit_command(npr::Command::AdvanceStep);
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then WorkflowCompleted was emitted.
        let has_completed = processed
            .iter()
            .any(|p| matches!(p.event, npr::Event::WorkflowCompleted));
        assert!(has_completed);
    }

    // --- Test 5: Workflow completion posts system message to chat ---

    #[test]
    fn workflow_completion_posts_system_message_to_chat() {
        // Given a two-step workflow advanced to completion.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(2),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // Complete and advance step-0.
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-0".to_owned(),
                resolved_outputs: HashMap::new(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);
        bus.submit_command(npr::Command::AdvanceStep);
        process_and_drain_events(&mut bus, &mut state, &services);

        // Complete and advance step-1 (final step).
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-1".to_owned(),
                resolved_outputs: HashMap::new(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);
        bus.submit_command(npr::Command::AdvanceStep);
        process_and_drain_events(&mut bus, &mut state, &services);

        // Then a system message was posted to the chat log.
        let history = state.active_session().history();
        let system_entries: Vec<_> = history
            .iter()
            .filter(|e| matches!(e.kind, npr::ChatEntryKind::System(_)))
            .collect();
        assert_eq!(system_entries.len(), 1);

        // And the message contains the workflow name and step count.
        if let npr::ChatEntryKind::System(ref text) = system_entries[0].kind {
            assert!(text.contains("test-workflow"));
            assert!(text.contains("2 steps finished"));
        } else {
            panic!("expected System entry");
        }
    }

    // --- Test 6: AdvanceStep does nothing when no workflow ---

    #[test]
    fn advance_step_does_nothing_when_no_workflow() {
        // Given a bus with WorkflowHandler but no workflow loaded.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When advancing with no workflow.
        bus.submit_command(npr::Command::AdvanceStep);
        bus.process_commands(&mut state, &services);

        // Then no crash and no workflow exists.
        assert!(!state.active_session().has_workflow());
    }

    // --- Test 7: JumpToStep activates target and marks downstream stale ---

    #[test]
    fn jump_to_step_activates_target_and_marks_downstream_stale() {
        // Given a multi-step workflow advanced past step 0.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(3),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // Advance to step 1.
        bus.submit_command(npr::Command::AdvanceStep);
        process_and_drain_events(&mut bus, &mut state, &services);

        // When jumping back to step 0.
        bus.submit_command(npr::Command::JumpToStep {
            payload: JumpToStep {
                step_id: "step-0".to_owned(),
            },
        });
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then step-0 is active and downstream steps are stale.
        assert_eq!(
            state
                .active_session()
                .workflow()
                .unwrap()
                .active_step
                .as_deref(),
            Some("step-0")
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::Active
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-1"].status,
            StepStatus::Stale
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-2"].status,
            StepStatus::Stale
        );

        // And StepStale and StepStarted events were emitted.
        assert!(processed.len() >= 2);
        assert!(matches!(
            &processed[0].event,
            npr::Event::StepStale { payload } if payload.step_ids.contains(&"step-1".to_owned())
        ));
    }

    // --- Test 8: JumpToStep emits StepStarted only (no StepAwaitingInput) ---

    #[test]
    fn jump_to_step_emits_step_started_without_awaiting_input() {
        // Given a workflow where step-0 is completed and step-1 is active.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow_with_input(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);

        // Complete step-0 and advance to step-1.
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-0".to_owned(),
                resolved_outputs: HashMap::new(),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);
        bus.submit_command(npr::Command::AdvanceStep);
        process_and_drain_events(&mut bus, &mut state, &services);

        // When jumping back to step-0.
        bus.submit_command(npr::Command::JumpToStep {
            payload: JumpToStep {
                step_id: "step-0".to_owned(),
            },
        });
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then StepStarted was emitted but NOT StepAwaitingInput.
        let has_started = processed.iter().any(|p| {
            matches!(
                &p.event,
                npr::Event::StepStarted { payload } if payload.step_id == "step-0"
            )
        });
        let has_awaiting = processed
            .iter()
            .any(|p| matches!(&p.event, npr::Event::StepAwaitingInput { .. }));
        assert!(has_started);
        assert!(!has_awaiting);
    }

    // --- Test 9: CompleteStep sets AwaitingInput and emits event ---

    // --- Test 10: AbortWorkflow removes state ---

    #[test]
    fn abort_workflow_removes_state() {
        // Given a loaded workflow.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(2),
            },
        });
        bus.process_commands(&mut state, &services);
        assert!(state.active_session().has_workflow());

        // When aborting the workflow.
        bus.submit_command(npr::Command::AbortWorkflow);
        bus.process_commands(&mut state, &services);

        // Then the workflow state is gone.
        assert!(!state.active_session().has_workflow());
    }

    // --- Test 11: WorkflowState persists through serde ---

    #[test]
    fn workflow_state_persists_through_serde() {
        // Given a workflow state in mid-progress.
        let def = make_workflow(3);
        let mut ws = WorkflowState::new(def);
        ws.start().unwrap();

        // When serializing and deserializing.
        let json = serde_json::to_string(&ws).unwrap();
        let back: WorkflowState = serde_json::from_str(&json).unwrap();

        // Then the state is equivalent.
        assert_eq!(ws.active_step, back.active_step);
        assert_eq!(ws.steps.len(), back.steps.len());
        assert_eq!(ws.steps["step-0"].status, back.steps["step-0"].status);
    }

    // --- Test 12: CompleteStep records outputs ---

    #[test]
    fn complete_step_sets_awaiting_input_and_emits_event() {
        // Given a loaded workflow with step-0 active.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(2),
            },
        });
        process_and_drain_events(&mut bus, &mut state, &services);
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::Active
        );

        // When completing step-0 with resolved outputs.
        let outputs = HashMap::from([("result".to_owned(), "42".to_owned())]);
        bus.submit_command(npr::Command::CompleteStep {
            payload: CompleteStep {
                step_id: "step-0".to_owned(),
                resolved_outputs: outputs,
            },
        });
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then step-0 is AwaitingInput (not Completed) with stored outputs.
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"].status,
            StepStatus::AwaitingInput
        );
        assert_eq!(
            state.active_session().workflow().unwrap().steps["step-0"]
                .resolved_outputs
                .get("result"),
            Some(&"42".to_owned())
        );

        // And StepAwaitingInput event was emitted.
        let has_awaiting = processed.iter().any(|p| {
            matches!(
                &p.event,
                npr::Event::StepAwaitingInput { payload } if payload.step_id == "step-0"
            )
        });
        assert!(has_awaiting);
    }

    // --- Test 13: StepStarted event includes context ---

    #[test]
    fn step_started_event_includes_context() {
        // Given a bus with WorkflowHandler registered.
        let mut bus = setup_bus();
        let services = test_utils::test_services();

        // When loading a workflow.
        bus.submit_command(npr::Command::LoadWorkflow {
            payload: LoadWorkflow {
                definition: make_workflow(2),
            },
        });
        let mut state = AppState::default();
        let processed = process_and_drain_events(&mut bus, &mut state, &services);

        // Then StepStarted has enriched context.
        let started = processed.iter().find_map(|p| match &p.event {
            npr::Event::StepStarted { payload } => Some((**payload).clone()),
            _ => None,
        });
        assert!(started.is_some());
        let s = started.expect("found StepStarted");
        assert_eq!(s.step_id, "step-0");
        assert_eq!(s.instructions, "Instructions for step 0");
        assert!(!s.requires_user_input);
        assert!(!s.checkpoint);
        assert!(s.completed_outputs.is_empty());
    }
}
