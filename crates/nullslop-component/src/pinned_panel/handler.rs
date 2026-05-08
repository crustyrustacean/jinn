//! Bus handler for pinned panel commands.
//!
//! Handles selection navigation, unpin, and position changes within the pinned
//! context panel. Panel management commands (Toggle/Open/Close) are NOT handled
//! here — they are TuiApp-level commands (like `WorkflowTogglePane`), handled in
//! `app.rs::route_command`.

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol::ChatEntryId;
use nullslop_protocol::CommandAction;
use nullslop_protocol::PinPosition;
use nullslop_protocol::SessionId;
use nullslop_protocol::context::{PinChatEntry, UnpinChatEntry};
use nullslop_protocol::system::{
    PinnedPanelPinBottom, PinnedPanelPinCycle, PinnedPanelPinRelative, PinnedPanelPinTop,
    PinnedPanelSelectDown, PinnedPanelSelectUp, PinnedPanelUnpin,
};
use nullslop_services::Services;

use crate::AppState;
use crate::app_state::pin_sort_key;

define_handler! {
    pub(crate) struct PinnedPanelHandler;

    commands {
        PinnedPanelSelectDown: on_select_down,
        PinnedPanelSelectUp: on_select_up,
        PinnedPanelUnpin: on_unpin,
        PinnedPanelPinTop: on_pin_top,
        PinnedPanelPinBottom: on_pin_bottom,
        PinnedPanelPinRelative: on_pin_relative,
        PinnedPanelPinCycle: on_pin_cycle,
    }

    events {}
}

/// Resolves the currently selected entry's ID and the active session ID.
/// Returns `None` if no pinned entries exist.
fn resolve_selected_entry_id(state: &AppState) -> Option<(SessionId, ChatEntryId)> {
    let sorted_ids = state.sorted_pinned_ids();
    let index = state.pinned_panel.selection_index(&sorted_ids);
    let session_id = state.active_session.clone();

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

    let entry = pinned.get(index)?;
    Some((session_id, entry.id.clone()))
}

/// Cycles pin position: TOP → BOTTOM → RELATIVE → TOP.
fn cycle_position(pos: PinPosition) -> PinPosition {
    match pos {
        PinPosition::Top => PinPosition::Bottom,
        PinPosition::Bottom => PinPosition::Relative,
        PinPosition::Relative => PinPosition::Top,
    }
}

impl PinnedPanelHandler {
    /// Moves the pinned panel selection down one entry.
    fn on_select_down(
        _cmd: &PinnedPanelSelectDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let sorted_ids = ctx.state.sorted_pinned_ids();
        ctx.state.pinned_panel.select_next(&sorted_ids);
        CommandAction::Continue
    }

    /// Moves the pinned panel selection up one entry.
    fn on_select_up(
        _cmd: &PinnedPanelSelectUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let sorted_ids = ctx.state.sorted_pinned_ids();
        ctx.state.pinned_panel.select_prev(&sorted_ids);
        CommandAction::Continue
    }

    /// Unpins the currently selected pinned entry from the pinned panel.
    ///
    /// Looks up the selected pinned entry by index (resolved from sorted IDs)
    /// and submits an `UnpinChatEntry` command via the Out buffer.
    fn on_unpin(
        _cmd: &PinnedPanelUnpin,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if let Some((session_id, entry_id)) = resolve_selected_entry_id(ctx.state) {
            ctx.out
                .submit_command(nullslop_protocol::Command::UnpinChatEntry {
                    payload: UnpinChatEntry {
                        session_id,
                        entry_id,
                    },
                });
        }
        CommandAction::Continue
    }

    /// Sets the selected pinned entry's position to TOP.
    fn on_pin_top(
        _cmd: &PinnedPanelPinTop,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if let Some((session_id, entry_id)) = resolve_selected_entry_id(ctx.state) {
            ctx.out
                .submit_command(nullslop_protocol::Command::PinChatEntry {
                    payload: PinChatEntry {
                        session_id,
                        entry_id,
                        position: PinPosition::Top,
                    },
                });
        }
        CommandAction::Continue
    }

    /// Sets the selected pinned entry's position to BOTTOM.
    fn on_pin_bottom(
        _cmd: &PinnedPanelPinBottom,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if let Some((session_id, entry_id)) = resolve_selected_entry_id(ctx.state) {
            ctx.out
                .submit_command(nullslop_protocol::Command::PinChatEntry {
                    payload: PinChatEntry {
                        session_id,
                        entry_id,
                        position: PinPosition::Bottom,
                    },
                });
        }
        CommandAction::Continue
    }

    /// Sets the selected pinned entry's position to RELATIVE.
    fn on_pin_relative(
        _cmd: &PinnedPanelPinRelative,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if let Some((session_id, entry_id)) = resolve_selected_entry_id(ctx.state) {
            ctx.out
                .submit_command(nullslop_protocol::Command::PinChatEntry {
                    payload: PinChatEntry {
                        session_id,
                        entry_id,
                        position: PinPosition::Relative,
                    },
                });
        }
        CommandAction::Continue
    }

    /// Cycles the selected pinned entry's position: TOP → BOTTOM → RELATIVE → TOP.
    fn on_pin_cycle(
        _cmd: &PinnedPanelPinCycle,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let sorted_ids = ctx.state.sorted_pinned_ids();
        let index = ctx.state.pinned_panel.selection_index(&sorted_ids);

        let mut pinned = ctx.state.active_session().pinned_entries();
        pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

        let Some(entry) = pinned.get(index) else {
            return CommandAction::Continue;
        };

        let current = entry.pin_position.unwrap_or(PinPosition::Relative);
        let next = cycle_position(current);
        let session_id = ctx.state.active_session.clone();
        let entry_id = entry.id.clone();

        ctx.out
            .submit_command(nullslop_protocol::Command::PinChatEntry {
                payload: PinChatEntry {
                    session_id,
                    entry_id,
                    position: next,
                },
            });
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use nullslop_component_core::Bus;
    use nullslop_protocol::ChatEntry;
    use nullslop_protocol::Command;
    use nullslop_protocol::PinPosition;
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

    #[rstest::rstest]
    fn select_down_increments_index() {
        // Given a bus with PinnedPanelHandler and a session with 3 pinned entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(3);

        // When processing a PinnedPanelSelectDown command.
        bus.submit_command(Command::PinnedPanelSelectDown);
        bus.process_commands(&mut state, &services);

        // Then the selection has moved to the second sorted ID.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(state.pinned_panel.selected_id(), Some(&sorted_ids[1]));
    }

    #[rstest::rstest]
    fn select_up_decrements_index() {
        // Given a bus with PinnedPanelHandler and a session with 3 pinned entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(3);
        // Move selection to index 1.
        let sorted_ids = state.sorted_pinned_ids();
        state.pinned_panel.select_next(&sorted_ids);
        assert_eq!(state.pinned_panel.selected_id(), Some(&sorted_ids[1]));

        // When processing a PinnedPanelSelectUp command.
        bus.submit_command(Command::PinnedPanelSelectUp);
        bus.process_commands(&mut state, &services);

        // Then the selection has moved back to the first sorted ID.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(state.pinned_panel.selected_id(), Some(&sorted_ids[0]));
    }

    #[rstest::rstest]
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
        let has_unpin = processed
            .iter()
            .any(|p| matches!(&p.command, Command::UnpinChatEntry { .. }));
        assert!(has_unpin, "expected UnpinChatEntry command to be submitted");
    }

    // --- Position-set tests ---

    #[rstest::rstest]
    fn pin_top_submits_pin_command() {
        // Given a bus with PinnedPanelHandler and a session with a pinned entry.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(1);

        // When processing a PinnedPanelPinTop command.
        bus.submit_command(Command::PinnedPanelPinTop);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Top.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Top));
    }

    #[rstest::rstest]
    fn pin_bottom_submits_pin_command() {
        // Given a bus with PinnedPanelHandler and a session with a pinned entry.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(1);

        // When processing a PinnedPanelPinBottom command.
        bus.submit_command(Command::PinnedPanelPinBottom);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Bottom.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pin_relative_submits_pin_command() {
        // Given a bus with PinnedPanelHandler and a session with a pinned entry.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = state_with_pinned(1);

        // When processing a PinnedPanelPinRelative command.
        bus.submit_command(Command::PinnedPanelPinRelative);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Relative.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Relative));
    }

    // --- Cycle position tests ---

    #[rstest::rstest]
    fn pin_cycle_rotates_top_to_bottom() {
        // Given a pinned entry at TOP.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        // Select it.
        let sorted_ids = state.sorted_pinned_ids();
        state.pinned_panel.select_by_id(sorted_ids[0].clone());

        // When processing a PinnedPanelPinCycle command.
        bus.submit_command(Command::PinnedPanelPinCycle);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Bottom.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pin_cycle_rotates_bottom_to_relative() {
        // Given a pinned entry at BOTTOM.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Bottom);
        let sorted_ids = state.sorted_pinned_ids();
        state.pinned_panel.select_by_id(sorted_ids[0].clone());

        // When processing a PinnedPanelPinCycle command.
        bus.submit_command(Command::PinnedPanelPinCycle);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Relative.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Relative));
    }

    #[rstest::rstest]
    fn pin_cycle_rotates_relative_to_top() {
        // Given a pinned entry at RELATIVE.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Relative);
        let sorted_ids = state.sorted_pinned_ids();
        state.pinned_panel.select_by_id(sorted_ids[0].clone());

        // When processing a PinnedPanelPinCycle command.
        bus.submit_command(Command::PinnedPanelPinCycle);
        bus.process_commands(&mut state, &services);

        // Then a PinChatEntry command was submitted with PinPosition::Top.
        let processed = bus.drain_processed_commands();
        let position = processed.iter().find_map(|p| {
            if let Command::PinChatEntry { payload } = &p.command {
                Some(payload.position)
            } else {
                None
            }
        });
        assert_eq!(position, Some(PinPosition::Top));
    }

    // --- Noop tests ---

    #[rstest::rstest]
    fn pin_top_is_noop_when_empty() {
        // Given a bus with PinnedPanelHandler and an empty session.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing a PinnedPanelPinTop command.
        bus.submit_command(Command::PinnedPanelPinTop);
        bus.process_commands(&mut state, &services);

        // Then no PinChatEntry command was submitted.
        let processed = bus.drain_processed_commands();
        let has_pin = processed
            .iter()
            .any(|p| matches!(&p.command, Command::PinChatEntry { .. }));
        assert!(
            !has_pin,
            "expected no PinChatEntry command when pinned panel is empty"
        );
    }

    #[rstest::rstest]
    fn pin_cycle_is_noop_when_empty() {
        // Given a bus with PinnedPanelHandler and an empty session.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing a PinnedPanelPinCycle command.
        bus.submit_command(Command::PinnedPanelPinCycle);
        bus.process_commands(&mut state, &services);

        // Then no PinChatEntry command was submitted.
        let processed = bus.drain_processed_commands();
        let has_pin = processed
            .iter()
            .any(|p| matches!(&p.command, Command::PinChatEntry { .. }));
        assert!(
            !has_pin,
            "expected no PinChatEntry command when pinned panel is empty"
        );
    }
}
