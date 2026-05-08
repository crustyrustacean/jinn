//! Bus handler for pinned panel commands.
//!
//! Handles selection navigation and unpin within the pinned context panel.
//! Panel management commands (Toggle/Open/Close) are NOT handled here — they are
//! TuiApp-level commands (like `WorkflowTogglePane`), handled in `app.rs::route_command`.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol::CommandAction;
use nullslop_protocol::context::UnpinChatEntry;
use nullslop_protocol::system::{PinnedPanelSelectDown, PinnedPanelSelectUp, PinnedPanelUnpin};
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct PinnedPanelHandler;

    commands {
        PinnedPanelSelectDown: on_select_down,
        PinnedPanelSelectUp: on_select_up,
        PinnedPanelUnpin: on_unpin,
    }

    events {}
}

impl PinnedPanelHandler {
    /// Moves the pinned panel selection down one entry.
    fn on_select_down(
        _cmd: &PinnedPanelSelectDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let pinned_count = ctx.state.active_session().pinned_entries().len();
        ctx.state.pinned_panel.select_next(pinned_count);
        CommandAction::Continue
    }

    /// Moves the pinned panel selection up one entry.
    fn on_select_up(
        _cmd: &PinnedPanelSelectUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.pinned_panel.select_prev();
        CommandAction::Continue
    }

    /// Unpins the currently selected pinned entry from the pinned panel.
    ///
    /// Looks up the selected pinned entry by index and submits an
    /// `UnpinChatEntry` command via the Out buffer.
    fn on_unpin(
        _cmd: &PinnedPanelUnpin,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let session_id = ctx.state.active_session.clone();
        let pinned = ctx.state.active_session().pinned_entries();
        let index = ctx.state.pinned_panel.selection_index();
        let Some(entry) = pinned.get(index) else {
            return CommandAction::Continue;
        };
        let entry_id = entry.id.clone();
        ctx.out.submit_command(nullslop_protocol::Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id,
                entry_id,
            },
        });
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use nullslop_component_core::Bus;
    use nullslop_protocol::Command;
    use nullslop_protocol::PinPosition;
    use nullslop_protocol::ChatEntry;
    use nullslop_services::Services;

    use super::*;
    use crate::AppState;
    use crate::test_utils;

    fn setup_bus() -> Bus<AppState, Services> {
        let mut bus: Bus<AppState, Services> = Bus::new();
        PinnedPanelHandler.register(&mut bus);
        bus
    }

    fn state_with_pinned(count: usize) -> AppState {
        let mut state = AppState::default();
        for i in 0..count {
            let entry = ChatEntry::user(format!("entry {i}"));
            let entry_id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            state
                .active_session_mut()
                .pin_entry(&entry_id, PinPosition::Top);
        }
        state
    }

    #[test]
    fn select_down_increments_index() {
        // Given a bus with PinnedPanelHandler and a session with 3 pinned entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(3);

        // When processing a PinnedPanelSelectDown command.
        bus.submit_command(Command::PinnedPanelSelectDown);
        bus.process_commands(&mut state, &services);

        // Then the selection index is 1.
        assert_eq!(state.pinned_panel.selection_index(), 1);
    }

    #[test]
    fn select_up_decrements_index() {
        // Given a bus with PinnedPanelHandler and a session with 3 pinned entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(3);
        state.pinned_panel.select_next(3);
        assert_eq!(state.pinned_panel.selection_index(), 1);

        // When processing a PinnedPanelSelectUp command.
        bus.submit_command(Command::PinnedPanelSelectUp);
        bus.process_commands(&mut state, &services);

        // Then the selection index is 0.
        assert_eq!(state.pinned_panel.selection_index(), 0);
    }

    #[test]
    fn unpin_submits_unpin_command() {
        // Given a bus with PinnedPanelHandler and a session with 2 pinned entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(2);

        // When processing a PinnedPanelUnpin command.
        bus.submit_command(Command::PinnedPanelUnpin);
        bus.process_commands(&mut state, &services);

        // Then an UnpinChatEntry command was submitted.
        let processed = bus.drain_processed_commands();
        let has_unpin = processed.iter().any(|p| {
            matches!(&p.command, Command::UnpinChatEntry { .. })
        });
        assert!(has_unpin, "expected UnpinChatEntry command to be submitted");
    }
}
