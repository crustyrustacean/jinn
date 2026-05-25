//! Shared infrastructure — actor framework, services, core coordination, state.

pub mod actor;
pub mod actor_host;
pub mod app_info;
pub mod app_paths;
pub mod app_state;
#[cfg(test)]
mod app_state_tests;
pub mod core;
pub mod frontmatter;
pub mod services;
pub mod session_map;
pub mod state;
pub mod system_resource;
pub mod tui_signals;
pub mod ui_element;
pub mod ui_element_fake;
pub mod ui_registry;

/// Standard UI registry type for the nullslop application.
pub type AppUiRegistry = ui_registry::UiRegistry<app_state::AppState>;

/// Register all UI elements from every feature module.
///
/// Called once during application startup. Each feature module that provides
/// UI elements exposes a `register()` function that adds its elements to the registry.
pub fn register_all_ui_elements(registry: &mut AppUiRegistry) {
    crate::feat::ui::status_bar::register(registry);
    crate::feat::ui::chat_log::register(registry);
    crate::feat::provider::register(registry);
    crate::feat::chat_input::register(registry);
}
