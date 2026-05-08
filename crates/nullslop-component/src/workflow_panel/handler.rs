//! Bus handler for workflow panel commands.
//!
//! Processes selection navigation, detail toggle, and step action commands
//! (jump, approve). Selection commands mutate [`WorkflowPanelState`] directly.
//! Jump and approve commands submit workflow lifecycle commands via the `Out`
//! buffer so the existing workflow handler processes them.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol::CommandAction;
use nullslop_protocol::system::{
    WorkflowApproveStep, WorkflowRestartStep, WorkflowSelectDown, WorkflowSelectFirst,
    WorkflowSelectLast, WorkflowSelectUp, WorkflowToggleDetail,
};
use nullslop_protocol::workflow::JumpToStep;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct WorkflowPanelHandler;

    commands {
        WorkflowSelectDown: on_select_down,
        WorkflowSelectUp: on_select_up,
        WorkflowSelectFirst: on_select_first,
        WorkflowSelectLast: on_select_last,
        WorkflowRestartStep: on_restart_step,
        WorkflowApproveStep: on_approve_step,
        WorkflowToggleDetail: on_toggle_detail,
    }

    events {}
}

impl WorkflowPanelHandler {
    /// Moves the workflow panel selection down one step.
    fn on_select_down(
        _cmd: &WorkflowSelectDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let step_count = ctx
            .state
            .active_session()
            .workflow()
            .map_or(0, |w| w.definition.steps.len());
        ctx.state.workflow_panel.select_next(step_count);
        CommandAction::Continue
    }

    /// Moves the workflow panel selection up one step.
    fn on_select_up(
        _cmd: &WorkflowSelectUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.workflow_panel.select_prev();
        CommandAction::Continue
    }

    /// Moves the workflow panel selection to the first step.
    fn on_select_first(
        _cmd: &WorkflowSelectFirst,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.workflow_panel.select_first();
        CommandAction::Continue
    }

    /// Moves the workflow panel selection to the last step.
    fn on_select_last(
        _cmd: &WorkflowSelectLast,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let step_count = ctx
            .state
            .active_session()
            .workflow()
            .map_or(0, |w| w.definition.steps.len());
        ctx.state.workflow_panel.select_last(step_count);
        CommandAction::Continue
    }

    /// Restarts the currently selected workflow step.
    ///
    /// Submits a [`JumpToStep`] command via the `Out` buffer so the workflow
    /// handler re-runs the selected step and marks downstream steps stale.
    /// No-op when no workflow is active.
    fn on_restart_step(
        _cmd: &WorkflowRestartStep,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let Some(workflow) = ctx.state.active_session().workflow() else {
            return CommandAction::Continue;
        };

        let step_ids = workflow.step_order();
        let index = ctx.state.workflow_panel.selected_index();
        let Some(step_id) = step_ids.get(index) else {
            return CommandAction::Continue;
        };

        ctx.out
            .submit_command(nullslop_protocol::Command::JumpToStep {
                payload: JumpToStep {
                    step_id: step_id.clone(),
                },
            });
        CommandAction::Continue
    }

    /// Approves the currently active workflow step.
    ///
    /// Submits an [`AdvanceStep`] command to finalize the current step and
    /// advance to the next one. No-op when no workflow is active.
    fn on_approve_step(
        _cmd: &WorkflowApproveStep,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // Only approve if there's an active workflow.
        if ctx.state.active_session().workflow().is_none() {
            return CommandAction::Continue;
        }

        ctx.out
            .submit_command(nullslop_protocol::Command::AdvanceStep);
        CommandAction::Continue
    }

    /// Toggles the step detail view.
    fn on_toggle_detail(
        _cmd: &WorkflowToggleDetail,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.workflow_panel.toggle_detail();
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_component_core::Bus;
    use nullslop_protocol::Command;
    use nullslop_services::Services;
    use nullslop_workflow::{GuardExpr, ModelHint, StepDef, WorkflowDef};

    use super::*;
    use crate::AppState;
    use crate::test_utils;

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

    /// Sets up a bus with `WorkflowPanelHandler` registered.
    fn setup_bus() -> Bus<AppState, Services> {
        let mut bus: Bus<AppState, Services> = Bus::new();
        WorkflowPanelHandler.register(&mut bus);
        bus
    }

    /// Helper: process commands and drain the processed commands list.
    fn process_and_drain_commands(
        bus: &mut Bus<AppState, Services>,
        state: &mut AppState,
        services: &Services,
    ) -> Vec<nullslop_component_core::bus::ProcessedCommand> {
        bus.process_commands(state, services);
        bus.drain_processed_commands()
    }

    /// Loads a workflow into state and returns the state.
    fn load_workflow(step_count: usize) -> AppState {
        let def = make_workflow(step_count);
        let mut ws = nullslop_workflow::WorkflowState::new(def);
        ws.start().unwrap();
        let mut state = AppState::default();
        state.active_session_mut().set_workflow(ws);
        state
    }

    #[rstest::rstest]
    fn select_down_increments_index() {
        // Given a bus with WorkflowPanelHandler and a 3-step workflow.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(3);

        // When processing WorkflowSelectDown.
        bus.submit_command(Command::WorkflowSelectDown);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 1.
        assert_eq!(state.workflow_panel.selected_index(), 1);
    }

    #[rstest::rstest]
    fn select_up_decrements_index() {
        // Given a bus with WorkflowPanelHandler and a 3-step workflow at index 1.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(3);
        state.workflow_panel.select_next(3);

        // When processing WorkflowSelectUp.
        bus.submit_command(Command::WorkflowSelectUp);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 0.
        assert_eq!(state.workflow_panel.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_first_moves_to_zero() {
        // Given a bus with WorkflowPanelHandler and a 3-step workflow at index 2.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(3);
        state.workflow_panel.select_next(3);
        state.workflow_panel.select_next(3);
        assert_eq!(state.workflow_panel.selected_index(), 2);

        // When processing WorkflowSelectFirst.
        bus.submit_command(Command::WorkflowSelectFirst);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 0.
        assert_eq!(state.workflow_panel.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_last_moves_to_end() {
        // Given a bus with WorkflowPanelHandler and a 3-step workflow.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(3);

        // When processing WorkflowSelectLast.
        bus.submit_command(Command::WorkflowSelectLast);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 2.
        assert_eq!(state.workflow_panel.selected_index(), 2);
    }

    #[rstest::rstest]
    fn restart_step_submits_jump_command() {
        // Given a bus with WorkflowPanelHandler and a 3-step workflow at index 1.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(3);
        state.workflow_panel.select_next(3);

        // When processing WorkflowRestartStep.
        bus.submit_command(Command::WorkflowRestartStep);
        let processed = process_and_drain_commands(&mut bus, &mut state, &services);

        // Then a JumpToStep command was submitted via Out.
        let has_jump = processed.iter().any(|p| {
            matches!(
                &p.command,
                Command::JumpToStep { payload } if payload.step_id == "step-1"
            )
        });
        assert!(has_jump, "expected JumpToStep for step-1");
    }

    #[rstest::rstest]
    fn approve_step_submits_advance_command() {
        // Given a bus with WorkflowPanelHandler and a workflow.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(2);

        // When processing WorkflowApproveStep.
        bus.submit_command(Command::WorkflowApproveStep);
        let processed = process_and_drain_commands(&mut bus, &mut state, &services);

        // Then an AdvanceStep command was submitted via Out.
        let has_advance = processed
            .iter()
            .any(|p| matches!(&p.command, Command::AdvanceStep));
        assert!(has_advance, "expected AdvanceStep command");
    }

    #[rstest::rstest]
    fn toggle_detail_flips_state() {
        // Given a bus with WorkflowPanelHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = load_workflow(2);
        assert!(!state.workflow_panel.show_detail());

        // When processing WorkflowToggleDetail.
        bus.submit_command(Command::WorkflowToggleDetail);
        bus.process_commands(&mut state, &services);

        // Then detail is now shown.
        assert!(state.workflow_panel.show_detail());
    }

    #[rstest::rstest]
    fn commands_noop_without_active_workflow() {
        // Given a bus with WorkflowPanelHandler but no workflow.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing all navigation commands.
        bus.submit_command(Command::WorkflowSelectDown);
        bus.submit_command(Command::WorkflowSelectUp);
        bus.submit_command(Command::WorkflowSelectFirst);
        bus.submit_command(Command::WorkflowSelectLast);
        bus.submit_command(Command::WorkflowRestartStep);
        bus.submit_command(Command::WorkflowApproveStep);
        bus.submit_command(Command::WorkflowToggleDetail);
        let processed = process_and_drain_commands(&mut bus, &mut state, &services);

        // Then no crash occurs and no JumpToStep/AdvanceStep were submitted.
        assert_eq!(state.workflow_panel.selected_index(), 0);
        let has_jump_or_advance = processed.iter().any(|p| {
            matches!(
                &p.command,
                Command::JumpToStep { .. } | Command::AdvanceStep
            )
        });
        assert!(
            !has_jump_or_advance,
            "expected no JumpToStep or AdvanceStep when no workflow"
        );
    }
}
