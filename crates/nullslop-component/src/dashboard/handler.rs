//! Dashboard handler — listens to actor lifecycle events.
//!
//! Tracks which actors are starting up and which have finished starting,
//! updating the dashboard state accordingly.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol::CommandAction;
use nullslop_protocol::actor::{ActorStarted, ActorStarting};
use nullslop_protocol::system::{
    DashboardSelectDown, DashboardSelectFirst, DashboardSelectLast, DashboardSelectUp,
};
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct DashboardHandler;

    commands {
        DashboardSelectDown: on_select_down,
        DashboardSelectFirst: on_select_first,
        DashboardSelectLast: on_select_last,
        DashboardSelectUp: on_select_up,
    }

    events {
        ActorStarting: on_actor_starting,
        ActorStarted: on_actor_started,
    }
}

impl DashboardHandler {
    /// Records an actor as starting in the dashboard state.
    fn on_actor_starting(evt: &ActorStarting, ctx: &mut HandlerContext<'_, AppState, Services>) {
        ctx.state
            .dashboard
            .mark_starting(&evt.name, evt.description.clone());
    }

    /// Records an actor as running in the dashboard state.
    fn on_actor_started(evt: &ActorStarted, ctx: &mut HandlerContext<'_, AppState, Services>) {
        ctx.state
            .dashboard
            .mark_running(&evt.name, evt.description.clone());
    }

    /// Moves the dashboard selection down one entry.
    fn on_select_down(
        _cmd: &DashboardSelectDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.dashboard.select_next();
        CommandAction::Continue
    }

    /// Moves the dashboard selection up one entry.
    fn on_select_up(
        _cmd: &DashboardSelectUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.dashboard.select_prev();
        CommandAction::Continue
    }

    /// Moves the dashboard selection to the first entry.
    fn on_select_first(
        _cmd: &DashboardSelectFirst,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.dashboard.select_first();
        CommandAction::Continue
    }

    /// Moves the dashboard selection to the last entry.
    fn on_select_last(
        _cmd: &DashboardSelectLast,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.dashboard.select_last();
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use nullslop_component_core::Bus;
    use nullslop_protocol::Command;
    use nullslop_protocol::Event;
    use nullslop_protocol::actor::{ActorStarted, ActorStarting};
    use nullslop_services::Services;

    use super::*;
    use crate::AppState;
    use crate::dashboard::state::ActorStatus;
    use crate::test_utils;

    #[rstest::rstest]
    fn actor_starting_adds_with_starting_status() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);

        // When an ActorStarting event is processed.
        bus.submit_event(Event::ActorStarting {
            payload: ActorStarting {
                name: "actor-a".into(),
                description: None,
            },
        });
        let services = test_utils::test_services();
        let mut state = AppState::default();
        bus.process_events(&mut state, &services);

        // Then the actor is tracked with Starting status.
        let actors = state.dashboard.actors();
        assert_eq!(actors.len(), 1);
        assert_eq!(actors[0].name, "actor-a");
        assert_eq!(actors[0].status, ActorStatus::Starting);
    }

    #[rstest::rstest]
    fn actor_started_updates_to_running() {
        // Given a bus with DashboardHandler registered and an actor that is running.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState::default();
        state.dashboard.mark_starting("actor-a", None);

        // When an ActorStarted event is processed.
        bus.submit_event(Event::ActorStarted {
            payload: ActorStarted {
                name: "actor-a".into(),
                description: None,
            },
        });
        bus.process_events(&mut state, &services);

        // Then the actor is updated to Running status.
        let actors = state.dashboard.actors();
        assert_eq!(actors[0].name, "actor-a");
        assert_eq!(actors[0].status, ActorStatus::Running);
    }

    #[rstest::rstest]
    fn first_actor_tracked_with_status() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);

        // When receiving a status update for the first actor.
        bus.submit_event(Event::ActorStarting {
            payload: ActorStarting {
                name: "echo".into(),
                description: Some("Echo tool".into()),
            },
        });
        let services = test_utils::test_services();
        let mut state = AppState::default();
        bus.process_events(&mut state, &services);

        // Then the first actor is tracked.
        assert!(state.dashboard.actors().iter().any(|a| a.name == "echo"));
    }

    #[rstest::rstest]
    fn second_actor_tracked_in_order() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);

        bus.submit_event(Event::ActorStarting {
            payload: ActorStarting {
                name: "echo".into(),
                description: Some("Echo tool".into()),
            },
        });
        bus.submit_event(Event::ActorStarting {
            payload: ActorStarting {
                name: "llm".into(),
                description: Some("LLM provider".into()),
            },
        });
        let services = test_utils::test_services();
        let mut state = AppState::default();
        bus.process_events(&mut state, &services);

        // Then both actors are tracked in order.
        let names: Vec<&str> = state
            .dashboard
            .actors()
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, vec!["echo", "llm"]);
    }

    #[rstest::rstest]
    fn select_down_moves_selection() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState::default();
        state.dashboard.mark_starting("echo", None);
        state.dashboard.mark_starting("llm", None);

        // When processing a DashboardSelectDown command.
        bus.submit_command(Command::DashboardSelectDown);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 1.
        assert_eq!(state.dashboard.selected_index(), 1);
    }

    #[rstest::rstest]
    fn select_up_clamps_at_zero() {
        // Given a bus with DashboardHandler registered at index 0.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState::default();
        state.dashboard.mark_starting("echo", None);
        state.dashboard.mark_starting("llm", None);

        // When processing a DashboardSelectUp command.
        bus.submit_command(Command::DashboardSelectUp);
        bus.process_commands(&mut state, &services);

        // Then the selected index stays at 0.
        assert_eq!(state.dashboard.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_first_moves_to_index_zero() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState::default();
        state.dashboard.mark_starting("echo", None);
        state.dashboard.mark_starting("llm", None);
        state.dashboard.mark_starting("ctx", None);
        state.dashboard.select_next();
        state.dashboard.select_next();
        assert_eq!(state.dashboard.selected_index(), 2);

        // When processing a DashboardSelectFirst command.
        bus.submit_command(Command::DashboardSelectFirst);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 0.
        assert_eq!(state.dashboard.selected_index(), 0);
    }

    #[rstest::rstest]
    fn select_last_moves_to_last_index() {
        // Given a bus with DashboardHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        DashboardHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState::default();
        state.dashboard.mark_starting("echo", None);
        state.dashboard.mark_starting("llm", None);
        state.dashboard.mark_starting("ctx", None);

        // When processing a DashboardSelectLast command.
        bus.submit_command(Command::DashboardSelectLast);
        bus.process_commands(&mut state, &services);

        // Then the selected index is 2.
        assert_eq!(state.dashboard.selected_index(), 2);
    }
}
