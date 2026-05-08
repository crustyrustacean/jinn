//! Picker handler — processes picker commands dispatched by [`PickerKind`].
//!
//! All 7 `Picker*` commands are shared across every picker type. Each handler
//! method dispatches on [`AppState::active_picker_kind`] to route to the
//! correct [`SelectionState`] field.

use npr::CommandAction;
use npr::PickerKind;
use npr::SessionLoadRequested;
use npr::context::SwitchPromptStrategy;
use npr::provider::ProviderSwitch;
use npr::provider_picker::{
    PickerBackspace, PickerConfirm, PickerInsertChar, PickerMoveCursorLeft, PickerMoveCursorRight,
    PickerMoveDown, PickerMoveUp,
};
use npr::system::SetMode;
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_services::Services;

use crate::AppState;
use crate::provider_picker::entries::{load_provider_entries, sorted_entries};

define_handler! {
    pub(crate) struct PickerHandler;

    commands {
        PickerInsertChar: on_insert_char,
        PickerBackspace: on_backspace,
        PickerConfirm: on_confirm,
        PickerMoveUp: on_move_up,
        PickerMoveDown: on_move_down,
        PickerMoveCursorLeft: on_move_cursor_left,
        PickerMoveCursorRight: on_move_cursor_right,
    }

    events {}
}

impl PickerHandler {
    /// Inserts a character into the active picker's filter.
    fn on_insert_char(
        cmd: &PickerInsertChar,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.insert_char(cmd.ch),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.insert_char(cmd.ch);
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.insert_char(cmd.ch),
            Some(PickerKind::Session) => ctx.state.session_picker.insert_char(cmd.ch),
            None => {}
        }
        CommandAction::Continue
    }

    /// Deletes the last character from the active picker's filter.
    fn on_backspace(
        _cmd: &PickerBackspace,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.backspace(),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.backspace();
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.backspace(),
            Some(PickerKind::Session) => ctx.state.session_picker.backspace(),
            None => {}
        }
        CommandAction::Continue
    }

    /// Confirms the active picker selection, dispatching to kind-specific logic.
    fn on_confirm(
        _cmd: &PickerConfirm,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => Self::confirm_provider(ctx),
            Some(PickerKind::ContextAssembly) => Self::confirm_strategy(ctx),
            Some(PickerKind::Keymap) => Self::confirm_keymap(ctx),
            Some(PickerKind::Session) => Self::confirm_session(ctx),
            None => {}
        }
        CommandAction::Continue
    }

    /// Moves the active picker selection up.
    fn on_move_up(
        _cmd: &PickerMoveUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.move_up(PICKER_MAX_VISIBLE),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.move_up(PICKER_MAX_VISIBLE);
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.move_up(PICKER_MAX_VISIBLE),
            Some(PickerKind::Session) => ctx.state.session_picker.move_up(PICKER_MAX_VISIBLE),
            None => {}
        }
        CommandAction::Continue
    }

    /// Moves the active picker selection down.
    fn on_move_down(
        _cmd: &PickerMoveDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.move_down(PICKER_MAX_VISIBLE),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.move_down(PICKER_MAX_VISIBLE);
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.move_down(PICKER_MAX_VISIBLE),
            Some(PickerKind::Session) => ctx.state.session_picker.move_down(PICKER_MAX_VISIBLE),
            None => {}
        }
        CommandAction::Continue
    }

    /// Moves the active picker filter cursor left.
    fn on_move_cursor_left(
        _cmd: &PickerMoveCursorLeft,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.move_cursor_left(),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.move_cursor_left();
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.move_cursor_left(),
            Some(PickerKind::Session) => ctx.state.session_picker.move_cursor_left(),
            None => {}
        }
        CommandAction::Continue
    }

    /// Moves the active picker filter cursor right.
    fn on_move_cursor_right(
        _cmd: &PickerMoveCursorRight,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        match ctx.state.active_picker_kind {
            Some(PickerKind::Provider) => ctx.state.provider_picker.move_cursor_right(),
            Some(PickerKind::ContextAssembly) => {
                ctx.state.context_strategy_picker.move_cursor_right();
            }
            Some(PickerKind::Keymap) => ctx.state.keymap_picker.move_cursor_right(),
            Some(PickerKind::Session) => ctx.state.session_picker.move_cursor_right(),
            None => {}
        }
        CommandAction::Continue
    }

    /// Provider-specific confirm: switches provider and closes the picker.
    fn confirm_provider(ctx: &mut HandlerContext<'_, AppState, Services>) {
        let Some(entry) = ctx.state.provider_picker.selected_item() else {
            return;
        };
        if !entry.is_available {
            return;
        }
        let provider_id = entry.provider_id.clone();

        // Submit provider switch.
        ctx.out.submit_command(npr::Command::ProviderSwitch {
            payload: ProviderSwitch { provider_id },
        });

        // Close picker.
        ctx.out.submit_command(npr::Command::SetMode {
            payload: SetMode {
                mode: npr::Mode::Normal,
            },
        });
    }

    /// Strategy-specific confirm: switches strategy, updates sticky default, closes picker.
    fn confirm_strategy(ctx: &mut HandlerContext<'_, AppState, Services>) {
        let Some(entry) = ctx.state.context_strategy_picker.selected_item() else {
            return;
        };
        let strategy_id = entry.strategy_id.clone();
        let session_id = ctx.state.active_session.clone();

        // Update sticky default.
        ctx.state.set_default_strategy(strategy_id.clone());

        // Switch strategy for active session.
        ctx.out.submit_command(npr::Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id,
                strategy_id,
            },
        });

        // Close picker.
        ctx.out.submit_command(npr::Command::SetMode {
            payload: SetMode {
                mode: npr::Mode::Normal,
            },
        });
    }

    /// Keymap-specific confirm: submits the stored command and closes the picker.
    fn confirm_keymap(ctx: &mut HandlerContext<'_, AppState, Services>) {
        let Some(entry) = ctx.state.keymap_picker.selected_item() else {
            return;
        };
        let command = entry.command.clone();

        // Close picker first so that SetMode{Normal} clears active_picker_kind
        // before the stored command potentially opens another picker.
        ctx.out.submit_command(npr::Command::SetMode {
            payload: SetMode {
                mode: npr::Mode::Normal,
            },
        });

        // Then submit the stored command.
        ctx.out.submit_command(command);
    }

    /// Session-specific confirm: submits a load request and closes the picker.
    fn confirm_session(ctx: &mut HandlerContext<'_, AppState, Services>) {
        let Some(entry) = ctx.state.session_picker.selected_item() else {
            return;
        };
        let session_id = entry.session_id.clone();
        let byte_offset = entry.byte_offset;

        ctx.state.session_loading = true;

        // Submit event for the actor to pick up.
        ctx.out.submit_event(npr::Event::SessionLoadRequested {
            payload: SessionLoadRequested {
                session_id,
                byte_offset,
            },
        });

        // Close picker.
        ctx.out.submit_command(npr::Command::SetMode {
            payload: SetMode {
                mode: npr::Mode::Normal,
            },
        });
    }
}

/// Maximum number of visible result rows used for scroll clamping in the handler.
/// The actual visible rows are determined dynamically by the renderer based on
/// terminal height. This value is a generous upper bound so the handler's scroll
/// offset tracking stays reasonable.
const PICKER_MAX_VISIBLE: usize = 100;

/// Loads provider entries into the picker state, ready for display.
///
/// Reads from the provider registry and model cache, applies available-first
/// sorting and active-provider promotion, then stores the entries via
/// [`SelectionState::set_items`].
pub fn load_provider_picker_items(services: &Services, state: &mut AppState) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(&registry, &api_keys, state.model_cache.as_ref());
    let entries = sorted_entries(&all, "", &state.active_provider);
    state.provider_picker.set_items(entries);
}

#[cfg(test)]
mod tests {
    use crate::context_strategy_picker::entries::load_strategy_picker_items;
    use crate::test_utils;
    use crate::AppState;
    use nullslop_component_core::Bus;
    use nullslop_protocol::PickerKind;
    use nullslop_protocol::PromptStrategyId;
    use nullslop_services::Services;

    use super::PickerHandler;

    fn setup_bus() -> Bus<AppState, Services> {
        let mut bus: Bus<AppState, Services> = Bus::new();
        PickerHandler.register(&mut bus);
        crate::chat_input_box::ChatInputBoxHandler.register(&mut bus);
        bus
    }

    #[rstest::rstest]    fn confirm_strategy_updates_default() {
        // Given a bus with PickerHandler and ChatInputBoxHandler, and a loaded strategy picker.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Navigate to the second entry (sliding_window, after passthrough).
        state.context_strategy_picker.move_down(100);

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then default_strategy is updated to the selected entry's strategy.
        assert_ne!(
            state.default_strategy,
            PromptStrategyId::passthrough(),
            "default_strategy should have been updated from passthrough"
        );
    }

    #[rstest::rstest]    fn confirm_strategy_returns_to_normal_mode() {
        // Given a bus with PickerHandler and ChatInputBoxHandler, and a loaded strategy picker.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Navigate to the second entry (sliding_window, after passthrough).
        state.context_strategy_picker.move_down(100);

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then mode is back to Normal.
        assert_eq!(state.mode, nullslop_protocol::Mode::Normal);
    }

    #[rstest::rstest]    fn confirm_strategy_noop_when_no_selection() {
        // Given a bus with PickerHandler, an empty strategy picker, and ContextAssembly kind.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        // No items loaded — selected_item() returns None.

        let initial_strategy = state.default_strategy.clone();

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then no commands were submitted — default_strategy unchanged.
        assert_eq!(state.default_strategy, initial_strategy);
    }

    // --- ContextAssembly picker dispatch tests ---

    #[rstest::rstest]    fn picker_insert_char_updates_context_strategy_filter() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // When processing PickerInsertChar with 'p'.
        bus.submit_command(nullslop_protocol::Command::PickerInsertChar {
            payload: nullslop_protocol::provider_picker::PickerInsertChar { ch: 'p' },
        });
        bus.process_commands(&mut state, &services);

        // Then the context strategy picker filter contains "p".
        assert_eq!(state.context_strategy_picker.filter(), "p");
    }

    #[rstest::rstest]    fn picker_backspace_removes_from_context_strategy_filter() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Insert "pa" then backspace.
        state.context_strategy_picker.insert_char('p');
        state.context_strategy_picker.insert_char('a');
        assert_eq!(state.context_strategy_picker.filter(), "pa");

        // When processing PickerBackspace.
        bus.submit_command(nullslop_protocol::Command::PickerBackspace);
        bus.process_commands(&mut state, &services);

        // Then the filter is "p".
        assert_eq!(state.context_strategy_picker.filter(), "p");
    }

    #[rstest::rstest]    fn picker_move_up_decrements_context_strategy_selection() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Move down to index 1.
        state.context_strategy_picker.move_down(100);
        assert_eq!(state.context_strategy_picker.selection(), 1);

        // When processing PickerMoveUp.
        bus.submit_command(nullslop_protocol::Command::PickerMoveUp);
        bus.process_commands(&mut state, &services);

        // Then the selection is back to 0.
        assert_eq!(state.context_strategy_picker.selection(), 0);
    }

    #[rstest::rstest]    fn picker_move_down_increments_context_strategy_selection() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        assert_eq!(state.context_strategy_picker.selection(), 0);

        // When processing PickerMoveDown.
        bus.submit_command(nullslop_protocol::Command::PickerMoveDown);
        bus.process_commands(&mut state, &services);

        // Then the selection is incremented to 1.
        assert_eq!(state.context_strategy_picker.selection(), 1);
    }

    #[rstest::rstest]    fn picker_move_cursor_left_decrements_context_strategy_cursor() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Insert "ab" — cursor should be at position 2.
        state.context_strategy_picker.insert_char('a');
        state.context_strategy_picker.insert_char('b');
        assert_eq!(state.context_strategy_picker.cursor_pos(), 2);

        // When processing PickerMoveCursorLeft.
        bus.submit_command(nullslop_protocol::Command::PickerMoveCursorLeft);
        bus.process_commands(&mut state, &services);

        // Then the cursor is at position 1.
        assert_eq!(state.context_strategy_picker.cursor_pos(), 1);
    }

    #[rstest::rstest]    fn picker_move_cursor_right_increments_context_strategy_cursor() {
        // Given a bus with PickerHandler, and a loaded strategy picker in ContextAssembly mode.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Insert "ab", move left twice — cursor at 0.
        state.context_strategy_picker.insert_char('a');
        state.context_strategy_picker.insert_char('b');
        state.context_strategy_picker.move_cursor_left();
        state.context_strategy_picker.move_cursor_left();
        assert_eq!(state.context_strategy_picker.cursor_pos(), 0);

        // When processing PickerMoveCursorRight.
        bus.submit_command(nullslop_protocol::Command::PickerMoveCursorRight);
        bus.process_commands(&mut state, &services);

        // Then the cursor is at position 1.
        assert_eq!(state.context_strategy_picker.cursor_pos(), 1);
    }

    #[rstest::rstest]    fn confirm_strategy_updates_sticky_default() {
        // Given a bus with handlers, loaded strategy picker on sliding_window (index 1).
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::ContextAssembly),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        load_strategy_picker_items(&services, &mut state);

        // Default is passthrough initially.
        assert_eq!(state.default_strategy(), &PromptStrategyId::passthrough());

        // Navigate to sliding_window (index 1).
        state.context_strategy_picker.move_down(100);

        // When confirming the selection.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then default_strategy is updated to sliding_window.
        assert_eq!(state.default_strategy(), &PromptStrategyId::sliding_window());
    }

    // --- Keymap picker dispatch tests ---

    /// Helper to create keymap entries for testing.
    fn keymap_entries() -> Vec<crate::keymap_picker::KeymapEntry> {
        use crate::keymap_picker::KeymapEntry;
        vec![
            KeymapEntry {
                key_sequence: "q".to_owned(),
                description: "quit".to_owned(),
                scope: "Normal".to_owned(),
                category: "General".to_owned(),
                command: nullslop_protocol::Command::Quit,
                search_text: "q quit".to_owned(),
            },
            KeymapEntry {
                key_sequence: "gg".to_owned(),
                description: "scroll to top".to_owned(),
                scope: "Normal".to_owned(),
                category: "Navigation".to_owned(),
                command: nullslop_protocol::Command::ScrollToTop,
                search_text: "gg scroll to top".to_owned(),
            },
        ]
    }

    #[rstest::rstest]    fn picker_insert_char_updates_keymap_filter() {
        // Given a bus with PickerHandler, and a keymap picker with entries.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::Keymap),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        state.keymap_picker.set_items(keymap_entries());

        // When processing PickerInsertChar with 'q'.
        bus.submit_command(nullslop_protocol::Command::PickerInsertChar {
            payload: nullslop_protocol::provider_picker::PickerInsertChar { ch: 'q' },
        });
        bus.process_commands(&mut state, &services);

        // Then the keymap picker filter contains "q".
        assert_eq!(state.keymap_picker.filter(), "q");
    }

    #[rstest::rstest]    fn confirm_keymap_closes_picker() {
        // Given a bus with PickerHandler, and a keymap picker with entries on "gg".
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::Keymap),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        state.keymap_picker.set_items(keymap_entries());

        // Navigate to "gg" (index 1).
        state.keymap_picker.move_down(100);

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then mode is back to Normal (picker closed).
        assert_eq!(state.mode, nullslop_protocol::Mode::Normal);
    }

    #[rstest::rstest]    fn confirm_keymap_submits_command() {
        // Given a bus with PickerHandler, and a keymap picker with entries on "gg".
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::Keymap),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        state.keymap_picker.set_items(keymap_entries());

        // Navigate to "gg" (index 1).
        state.keymap_picker.move_down(100);

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then the selected command (ScrollToTop) was dispatched, not quit.
        assert!(!state.should_quit, "ScrollToTop should not quit");
    }

    #[rstest::rstest]    fn picker_confirm_keymap_noop_when_no_selection() {
        // Given a bus with PickerHandler, and an empty keymap picker.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::Keymap),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        // No items loaded — selected_item() returns None.

        // When processing PickerConfirm.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then mode remains Picker (nothing happened).
        assert_eq!(state.mode, nullslop_protocol::Mode::Picker);
    }

    #[rstest::rstest]    fn picker_confirm_keymap_opens_another_picker() {
        // Given a bus with PickerHandler and OpenPickerHandler, and a keymap
        // picker with an entry whose command is OpenPicker { kind: Provider }.
        use crate::keymap_picker::KeymapEntry;

        let mut bus = setup_bus();
        crate::open_picker_handler::OpenPickerHandler.register(&mut bus);
        let services = test_utils::test_services();
        let mut state = AppState {
            active_picker_kind: Some(PickerKind::Keymap),
            mode: nullslop_protocol::Mode::Picker,
            ..AppState::default()
        };
        state.keymap_picker.set_items(vec![KeymapEntry {
            key_sequence: "gmp".to_owned(),
            description: "open provider picker".to_owned(),
            scope: "Normal".to_owned(),
            category: "Model".to_owned(),
            command: nullslop_protocol::Command::OpenPicker {
                payload: nullslop_protocol::system::OpenPicker {
                    kind: PickerKind::Provider,
                },
            },
            search_text: "gmp open provider picker".to_owned(),
        }]);

        // When confirming the entry.
        bus.submit_command(nullslop_protocol::Command::PickerConfirm);
        bus.process_commands(&mut state, &services);

        // Then the provider picker is open (mode is Picker, kind is Provider).
        assert_eq!(
            state.mode,
            nullslop_protocol::Mode::Picker,
            "mode should be Picker (provider picker open)"
        );
        assert_eq!(
            state.active_picker_kind,
            Some(PickerKind::Provider),
            "active_picker_kind should be Provider"
        );
    }
}
