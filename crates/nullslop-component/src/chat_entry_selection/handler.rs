//! Handler for chat entry selection commands.
//!
//! Delegates to [`ChatSessionState::select_next_entry`],
//! [`ChatSessionState::select_prev_entry`], and
//! [`ChatSessionState::clear_selection`].

use npr::chat_input::{ChatEntrySelectCancel, ChatEntrySelectNext, ChatEntrySelectPrev};
use npr::CommandAction;
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct ChatEntrySelectionHandler;

    commands {
        ChatEntrySelectNext: on_select_next,
        ChatEntrySelectPrev: on_select_prev,
        ChatEntrySelectCancel: on_select_cancel,
    }

    events {}
}

impl ChatEntrySelectionHandler {
    /// Moves the selection to the next chat entry.
    fn on_select_next(
        _cmd: &ChatEntrySelectNext,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_session_mut().select_next_entry();
        CommandAction::Continue
    }

    /// Moves the selection to the previous chat entry.
    fn on_select_prev(
        _cmd: &ChatEntrySelectPrev,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_session_mut().select_prev_entry();
        CommandAction::Continue
    }

    /// Clears the chat entry selection.
    fn on_select_cancel(
        _cmd: &ChatEntrySelectCancel,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_session_mut().clear_selection();
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use npr::ChatEntry;
    use npr::Command;
        use npr::chat_input::{ChatEntrySelectCancel, ChatEntrySelectNext, ChatEntrySelectPrev};
    use nullslop_component_core::Bus;
    use nullslop_protocol as npr;
    use nullslop_services::Services;

    use super::*;
    use crate::AppState;
    use crate::test_utils;

    #[test]
    fn select_next_command_increments_index() {
        // Given a bus with ChatEntrySelectionHandler registered and a session with 3 entries.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ChatEntrySelectionHandler.register(&mut bus);
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        let services = test_utils::test_services();

        // When processing ChatEntrySelectNext.
        bus.submit_command(Command::ChatEntrySelectNext {
            payload: ChatEntrySelectNext {
                session_id: state.active_session.clone(),
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the first entry is selected.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
    }

    #[test]
    fn select_prev_command_decrements_index() {
        // Given a bus with handler registered and selection at last entry.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ChatEntrySelectionHandler.register(&mut bus);
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::user("b"));
        state.active_session_mut().select_prev_entry(); // selects 1 (last)
        let services = test_utils::test_services();

        // When processing ChatEntrySelectPrev.
        bus.submit_command(Command::ChatEntrySelectPrev {
            payload: ChatEntrySelectPrev {
                session_id: state.active_session.clone(),
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the selection moves to index 0.
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
    }

    #[test]
    fn select_cancel_command_clears_selection() {
        // Given a bus with handler registered and an active selection.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ChatEntrySelectionHandler.register(&mut bus);
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("hello"));
        state.active_session_mut().select_next_entry();
        assert_eq!(state.active_session().selected_entry_index(), Some(0));
        let services = test_utils::test_services();

        // When processing ChatEntrySelectCancel.
        bus.submit_command(Command::ChatEntrySelectCancel {
            payload: ChatEntrySelectCancel {
                session_id: state.active_session.clone(),
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the selection is cleared.
        assert_eq!(state.active_session().selected_entry_index(), None);
    }
}
