//! `OpenPicker` handler — loads entries and enters Picker mode.
//!
//! Handles [`OpenPicker`] by setting [`active_picker_kind`], loading the
//! appropriate entries, resetting picker state, and switching to Picker mode.
//!
//! [`active_picker_kind`]: crate::AppState::active_picker_kind

use crate::AppState;
use crate::context_strategy_picker::entries::load_strategy_picker_items;
use crate::provider_picker::handler::load_provider_picker_items;
use crate::session_picker::entries::load_session_picker_items;
use npr::CommandAction;
use npr::PickerKind;
use npr::system::OpenPicker;
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_services::Services;

define_handler! {
    pub(crate) struct OpenPickerHandler;

    commands {
        OpenPicker: on_open_picker,
    }

    events {}
}

impl OpenPickerHandler {
    /// Processes the `OpenPicker` command: sets picker kind, loads entries, enters Picker mode.
    fn on_open_picker(
        cmd: &OpenPicker,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_picker_kind = Some(cmd.kind);

        match cmd.kind {
            PickerKind::Provider => {
                load_provider_picker_items(ctx.services, ctx.state);
                ctx.state.provider_picker.reset();
            }
            PickerKind::ContextAssembly => {
                load_strategy_picker_items(ctx.services, ctx.state);
                ctx.state.context_strategy_picker.reset();
            }
            PickerKind::Keymap => {
                ctx.state.keymap_picker.reset();
                ctx.state.keymap_picker_show_all = false;
            }
            PickerKind::Session => {
                load_session_picker_items(ctx.services, ctx.state);
                ctx.state.session_picker.reset();
            }
        }

        ctx.state.mode = npr::Mode::Picker;
        CommandAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use crate::AppState;
    use crate::test_utils;
    use nullslop_component_core::Bus;
    use nullslop_protocol::PickerKind;
    use nullslop_protocol::system::OpenPicker;
    use nullslop_services::Services;

    use super::OpenPickerHandler;

    fn setup_bus() -> Bus<AppState, Services> {
        let mut bus: Bus<AppState, Services> = Bus::new();
        OpenPickerHandler.register(&mut bus);
        bus
    }

    #[rstest::rstest]
    fn open_provider_sets_kind() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing OpenPicker { kind: Provider }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::Provider,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then active_picker_kind is Provider.
        assert_eq!(state.active_picker_kind, Some(PickerKind::Provider));
    }

    #[rstest::rstest]
    fn open_provider_sets_mode_to_picker() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing OpenPicker { kind: Provider }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::Provider,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then mode is Picker.
        assert_eq!(state.mode, nullslop_protocol::Mode::Picker);
    }

    #[rstest::rstest]
    fn open_context_assembly_sets_kind() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing OpenPicker { kind: ContextAssembly }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::ContextAssembly,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then active_picker_kind is ContextAssembly.
        assert_eq!(state.active_picker_kind, Some(PickerKind::ContextAssembly));
    }

    #[rstest::rstest]
    fn open_context_assembly_sets_mode_to_picker() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState::default();

        // When processing OpenPicker { kind: ContextAssembly }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::ContextAssembly,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then mode is Picker.
        assert_eq!(state.mode, nullslop_protocol::Mode::Picker);
    }

    #[rstest::rstest]
    fn open_keymap_sets_kind() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            keymap_picker_show_all: true,
            ..Default::default()
        };

        // When processing OpenPicker { kind: Keymap }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::Keymap,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then active_picker_kind is Keymap.
        assert_eq!(state.active_picker_kind, Some(PickerKind::Keymap));
    }

    #[rstest::rstest]
    fn open_keymap_sets_mode_to_picker() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        let mut state = AppState {
            keymap_picker_show_all: true,
            ..Default::default()
        };

        // When processing OpenPicker { kind: Keymap }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::Keymap,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then mode is Picker.
        assert_eq!(state.mode, nullslop_protocol::Mode::Picker);
    }

    #[rstest::rstest]
    fn open_keymap_resets_show_all() {
        // Given a bus with OpenPickerHandler.
        let mut bus = setup_bus();
        let services = test_utils::test_services();
        // Pre-populate with show_all flag set.
        let mut state = AppState {
            keymap_picker_show_all: true,
            ..Default::default()
        };

        // When processing OpenPicker { kind: Keymap }.
        bus.submit_command(nullslop_protocol::Command::OpenPicker {
            payload: OpenPicker {
                kind: PickerKind::Keymap,
            },
        });
        bus.process_commands(&mut state, &services);

        // Then keymap_picker_show_all is reset to false.
        assert!(!state.keymap_picker_show_all);
    }
}
