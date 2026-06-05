//! Builder for constructing a [`TuiApp`] with sensible defaults for tests.

use jinn_domain::ActorHostService;
use jinn_domain::AppCore;
use jinn_domain::AppUiRegistry;
use jinn_domain::feat::ui::sidebar::sidebar::Sidebar;

use super::{TuiApp, WhichKeyInstance};
use crate::config::TuiConfig;
use crate::selection::{SelectableRects, SelectionState};
use crate::suspend::Suspend;
use crate::{AppStatus, MsgHandler};

/// Builder for constructing a [`TuiApp`] with sensible defaults for tests.
///
/// All fields default to fake/noop implementations. Override only what the test needs.
///
/// See the tests in this crate for usage patterns.
#[derive(Default)]
pub struct TuiAppBuilder {
    /// Optional services override (defaults to fake services).
    services: Option<jinn_domain::Services>,
    /// Optional app state override (defaults to default state).
    state: Option<jinn_domain::AppState>,
    /// Optional plugins override (defaults to empty SyncPlugins).
    plugins: Option<jinn_plugin::SyncPlugins>,
}

impl TuiAppBuilder {
    /// Override the default services.
    #[must_use]
    pub fn services(mut self, services: jinn_domain::Services) -> Self {
        self.services = Some(services);
        self
    }

    /// Override the default app state.
    #[must_use]
    pub fn state(mut self, state: jinn_domain::AppState) -> Self {
        self.state = Some(state);
        self
    }

    /// Override the default plugins.
    #[must_use]
    pub fn plugins(mut self, plugins: jinn_plugin::SyncPlugins) -> Self {
        self.plugins = Some(plugins);
        self
    }

    /// Build the `TuiApp` with the configured overrides.
    pub fn build(self) -> TuiApp {
        let services = self.services.unwrap_or_default();
        let state = self.state.unwrap_or_default();

        let (sender, _receiver) = kanal::unbounded();
        let core = AppCore {
            state: jinn_domain::State::new(state),
            sender,
        };
        let fake_host =
            ActorHostService::new(std::sync::Arc::new(jinn_domain::FakeActorHost::new()));
        let mut ui_registry = AppUiRegistry::new();
        jinn_domain::register_all_ui_elements(&mut ui_registry);
        jinn_domain::feat::ui::status_bar::register(&mut ui_registry);
        jinn_domain::feat::ui::chat_log::register(&mut ui_registry);
        jinn_domain::feat::provider::register(&mut ui_registry);
        jinn_domain::feat::chat_input::register(&mut ui_registry);

        let initial_scope =
            crate::app::scope_for_focus(core.state.read().frontend.scope_stack.current());

        TuiApp {
            core,
            services,
            plugins: self.plugins.unwrap_or_else(jinn_plugin::SyncPlugins::empty),
            actor_host: fake_host,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(crate::keymap::init(), initial_scope),
            suspend: Suspend::new(),
            event_thread: None,
            status: AppStatus::Starting,
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config: TuiConfig::default(),
            sidebar: {
                let mut s = Sidebar::new();
                jinn_domain::feat::ui::sidebar::register_sections(&mut s);
                s
            },
        }
    }
}
