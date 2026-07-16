//! Builder for constructing a [`TuiApp`] with sensible defaults for tests.

use jinn_domain::AppCore;

use super::TuiApp;

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
    plugins: Option<jinn_wasm_host::SyncWasmPlugins>,
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
    pub fn plugins(mut self, plugins: jinn_wasm_host::SyncWasmPlugins) -> Self {
        self.plugins = Some(plugins);
        self
    }

    /// Build the `TuiApp` with the configured overrides.
    ///
    /// Delegates to [`crate::launch::launch_for_test`] so that the test path and
    /// the real launch path ([`crate::launch::launch`]) share a single keymap
    /// bootstrap site. This is what prevents the test/prod divergence that
    /// previously left plugin keybinds unbound in production.
    pub async fn build(self) -> TuiApp {
        let services = match self.services {
            Some(s) => s,
            None => jinn_domain::Services::new_fake().await,
        };
        let state = self.state.unwrap_or_default();

        let core = AppCore {
            state: jinn_domain::State::new(state),
            bridge: services.bridge.clone(),
        };
        let plugins = self
            .plugins
            .unwrap_or_else(jinn_wasm_host::SyncWasmPlugins::empty);

        crate::launch::launch_for_test(core, services, plugins)
    }
}
