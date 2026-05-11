//! Builder for constructing a [`TuiApp`] with sensible defaults for tests.

use nullslop_actor_host::ActorHostService;
use nullslop_component::AppUiRegistry;
use nullslop_core::AppCore;
use ratatui_spatial_splits::SplitManager;

use super::{PaneFocus, TuiApp, WhichKeyInstance};
use crate::config::TuiConfig;
use crate::scope::Scope;
use crate::selection::{SelectableRects, SelectionState};
use crate::suspend::Suspend;
use crate::{AppStatus, MsgHandler};

/// Builder for constructing a [`TuiApp`] with sensible defaults for tests.
///
/// All fields default to fake/noop implementations. Override only what the test needs.
///
/// ```ignore
/// let app = TuiApp::test_builder()
///     .services(custom_services)
///     .build();
/// ```
#[derive(Default)]
pub struct TuiAppBuilder {
    /// Optional services override (defaults to fake services).
    services: Option<nullslop_services::Services>,
    /// Optional app state override (defaults to default state).
    state: Option<nullslop_component::AppState>,
}

impl TuiAppBuilder {
    /// Override the default services.
    #[must_use]
    pub fn services(mut self, services: nullslop_services::Services) -> Self {
        self.services = Some(services);
        self
    }

    /// Override the default app state.
    #[must_use]
    pub fn state(mut self, state: nullslop_component::AppState) -> Self {
        self.state = Some(state);
        self
    }

    /// Build the `TuiApp` with the configured overrides.
    pub fn build(self) -> TuiApp {
        let services = self.services.unwrap_or_default();
        let state = self.state.unwrap_or_default();

        let (sender, _receiver) = kanal::unbounded();
        let core = AppCore {
            state: nullslop_component::State::new(state),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_protocol::CoreNotification>();
        let fake_host = ActorHostService::new(std::sync::Arc::new(
            nullslop_actor_host::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_all(&mut ui_registry);
        nsslice_status_bar::register(&mut ui_registry);
        nsslice_char_counter::register(&mut ui_registry);
        nsslice_dashboard::register(&mut ui_registry);
        nsslice_chat_log::register(&mut ui_registry);
        nsslice_provider::register(&mut ui_registry);
        nsslice_pinned_panel::register(&mut ui_registry);
        nsslice_chat_input_box::register(&mut ui_registry);

        TuiApp {
            core,
            services,
            actor_host: fake_host,
            core_receiver: core_rx,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(crate::keymap::init(), Scope::Normal),
            suspend: Suspend::new(),
            event_task: None,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config: TuiConfig::default(),
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            pinned_pane_visible: false,
            pinned_pane_id: None,
        }
    }
}
