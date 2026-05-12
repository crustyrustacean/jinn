//! Builder for constructing a [`TuiApp`] with sensible defaults for tests.

use nullslop_domain::ActorHostService;
use nullslop_domain::AppCore;
use nullslop_domain::AppUiRegistry;
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
    services: Option<nullslop_domain::Services>,
    /// Optional app state override (defaults to default state).
    state: Option<nullslop_domain::AppState>,
}

impl TuiAppBuilder {
    /// Override the default services.
    #[must_use]
    pub fn services(mut self, services: nullslop_domain::Services) -> Self {
        self.services = Some(services);
        self
    }

    /// Override the default app state.
    #[must_use]
    pub fn state(mut self, state: nullslop_domain::AppState) -> Self {
        self.state = Some(state);
        self
    }

    /// Build the `TuiApp` with the configured overrides.
    pub fn build(self) -> TuiApp {
        let services = self.services.unwrap_or_default();
        let state = self.state.unwrap_or_default();

        let (sender, _receiver) = kanal::unbounded();
        let core = AppCore {
            state: nullslop_domain::State::new(state),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_domain::CoreNotification>();
        let fake_host =
            ActorHostService::new(std::sync::Arc::new(nullslop_domain::FakeActorHost::new()));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_domain::register_all_ui_elements(&mut ui_registry);
        nullslop_domain::feat::ui::status_bar::register(&mut ui_registry);
        nullslop_domain::feat::ui::char_counter::register(&mut ui_registry);
        nullslop_domain::feat::dashboard::register(&mut ui_registry);
        nullslop_domain::feat::ui::chat_log::register(&mut ui_registry);
        nullslop_domain::feat::provider::register(&mut ui_registry);
        nullslop_domain::feat::chat_input::register(&mut ui_registry);

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
