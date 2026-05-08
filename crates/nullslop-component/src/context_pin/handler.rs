//! Handler for pin/unpin commands.
//!
//! Delegates to [`ChatSessionState::pin_entry`] and [`ChatSessionState::unpin_entry`].

use npr::context::{PinChatEntry, UnpinChatEntry};
use npr::CommandAction;
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct ContextPinHandler;

    commands {
        PinChatEntry: on_pin_entry,
        UnpinChatEntry: on_unpin_entry,
    }

    events {}
}

impl ContextPinHandler {
    /// Pins a chat entry at the specified position.
    fn on_pin_entry(
        cmd: &PinChatEntry,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state
            .session_mut(&cmd.session_id)
            .pin_entry(&cmd.entry_id, cmd.position);

        // Select the newly pinned entry.
        ctx.state.pinned_panel.select_by_id(cmd.entry_id.clone());
        CommandAction::Continue
    }

    /// Removes the pin from a chat entry.
    fn on_unpin_entry(
        cmd: &UnpinChatEntry,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // Resolve old index before mutation so we can clamp to nearest.
        let sorted_ids_before = ctx.state.sorted_pinned_ids();
        let old_index = ctx.state.pinned_panel.selection_index(&sorted_ids_before);

        ctx.state
            .session_mut(&cmd.session_id)
            .unpin_entry(&cmd.entry_id);

        // Clamp selection to nearest remaining entry.
        let sorted_ids_after = ctx.state.sorted_pinned_ids();
        ctx.state.pinned_panel.clamp_to_nearest(&sorted_ids_after, old_index);
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use npr::ChatEntry;
    use npr::ChatEntryId;
    use npr::Command;
    use npr::PinPosition;
    use npr::SessionId;
    use nullslop_component_core::Bus;
    use nullslop_protocol as npr;
    use nullslop_services::Services;

    use super::*;
    use crate::AppState;
    use crate::test_utils;

    /// Helper: create an AppState with a session containing a single user entry.
    ///
    /// Returns `(state, session_id, entry_id)`.
    fn state_with_entry(content: &str) -> (AppState, SessionId, ChatEntryId) {
        let mut state = AppState::default();
        let session_id = state.active_session.clone();
        let entry = ChatEntry::user(content);
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        (state, session_id, entry_id)
    }

    #[test]
    fn pin_command_sets_pin_position_on_matching_entry() {
        // Given a bus with ContextPinHandler registered and a session with one entry.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ContextPinHandler.register(&mut bus);
        let (mut state, session_id, entry_id) = state_with_entry("hello");
        let services = test_utils::test_services();

        // When processing a PinChatEntry command.
        bus.submit_command(Command::PinChatEntry {
            payload: PinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry_id.clone(),
                position: PinPosition::Top,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the entry has a pin position set.
        let entry = &state.active_session().history()[0];
        assert_eq!(entry.pin_position, Some(PinPosition::Top));

        // And the newly pinned entry is selected.
        assert_eq!(state.pinned_panel.selected_id(), Some(&entry_id));
    }

    #[test]
    fn unpin_command_clears_pin_position() {
        // Given a bus with ContextPinHandler registered and two pinned entries.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ContextPinHandler.register(&mut bus);
        let mut state = AppState::default();
        let session_id = state.active_session.clone();

        // Add two entries and pin both.
        let entry0 = ChatEntry::user("first");
        let entry0_id = entry0.id.clone();
        state.active_session_mut().push_entry(entry0);
        state.active_session_mut().pin_entry(&entry0_id, PinPosition::Top);

        let entry1 = ChatEntry::user("second");
        let entry1_id = entry1.id.clone();
        state.active_session_mut().push_entry(entry1);
        state.active_session_mut().pin_entry(&entry1_id, PinPosition::Top);

        // Select the first entry.
        state.pinned_panel.select_by_id(entry0_id.clone());
        let services = test_utils::test_services();

        // When processing an UnpinChatEntry command for the first entry.
        bus.submit_command(Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry0_id.clone(),
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the first entry's pin position is cleared.
        assert_eq!(state.active_session().history()[0].pin_position, None);

        // And selection moves to the nearest remaining entry (the second one).
        assert_eq!(state.pinned_panel.selected_id(), Some(&entry1_id));
    }

    #[test]
    fn pin_command_is_noop_for_nonexistent_entry() {
        // Given a bus with ContextPinHandler registered and a session with one entry.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ContextPinHandler.register(&mut bus);
        let (mut state, session_id, _) = state_with_entry("hello");
        let services = test_utils::test_services();

        // When pinning a non-existent entry ID.
        let missing_id = ChatEntryId::new();
        bus.submit_command(Command::PinChatEntry {
            payload: PinChatEntry {
                session_id: session_id.clone(),
                entry_id: missing_id.clone(),
                position: PinPosition::Relative,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the existing entry is unaffected.
        let entry = &state.active_session().history()[0];
        assert_eq!(entry.pin_position, None);

        // And the selection is set to the non-existent ID (handler doesn't validate).
        assert_eq!(state.pinned_panel.selected_id(), Some(&missing_id));
    }

    #[test]
    fn unpin_command_is_noop_for_nonexistent_entry() {
        // Given a bus with ContextPinHandler registered and a pinned entry.
        let mut bus: Bus<AppState, Services> = Bus::new();
        ContextPinHandler.register(&mut bus);
        let (mut state, session_id, entry_id) = state_with_entry("hello");
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        // Select the pinned entry.
        state.pinned_panel.select_by_id(entry_id.clone());
        let services = test_utils::test_services();

        // When unpinning a non-existent entry ID.
        let missing_id = ChatEntryId::new();
        bus.submit_command(Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id: session_id.clone(),
                entry_id: missing_id,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then the existing pinned entry is unaffected.
        let entry = &state.active_session().history()[0];
        assert_eq!(entry.pin_position, Some(PinPosition::Top));

        // And selection is still on the original entry.
        assert_eq!(state.pinned_panel.selected_id(), Some(&entry_id));
    }
}
