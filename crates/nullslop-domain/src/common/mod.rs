//! Shared infrastructure — actor framework, services, core coordination, state.

pub mod actor;
pub mod actor_host;
pub mod app_state;
pub mod core;
pub mod services;
pub mod state;
pub mod tui_signals;
pub mod ui_element;
pub mod ui_element_fake;
pub mod ui_registry;

/// Standard UI registry type for the nullslop application.
pub type AppUiRegistry = ui_registry::UiRegistry<app_state::AppState>;

/// Register all built-in UI elements.
///
/// Called once during application startup. After Phase 5+6, this only
/// registers UI elements — no bus handler registration.
pub fn register_all(registry: &mut AppUiRegistry) {
    register_tui_elements(registry);
}

/// Register only TUI elements.
///
/// Populates the UI element registry with all built-in elements.
/// After Phase 6, this is empty — all elements are registered by slice crates.
pub fn register_tui_elements(_registry: &mut AppUiRegistry) {}
