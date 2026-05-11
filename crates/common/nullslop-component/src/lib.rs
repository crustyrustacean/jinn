//! Built-in components for the nullslop application.
//!
//! A *component* is a self-contained piece of application behavior — it may handle
//! user actions, react to lifecycle events, render part of the interface, or any
//! combination of these. Each component owns a clear domain responsibility and is
//! wired into the application through [`register_all`], which is called once at
//! startup.
//!
//! The components in this crate together provide the core chat experience:
//! accepting user input, displaying conversation history, counting characters,
//! processing actor commands, and coordinating a clean shutdown.
//!
//! # Type aliases
//!
//! - [`AppUiRegistry`] — the standard UI element registry.

pub mod app_state;
pub mod prompt_template;
pub mod state;
pub mod tui_signals;

pub use app_state::{
    AppState, ContextAssemblyState, FrontendState, ProviderState, SessionState,
    ShutdownCoordinatorState,
};
pub use nsslice_chat_session_protocol::ChatSessionState;
pub use nsslice_chat_input_box_protocol::ChatInputBoxState;
pub use nsslice_dashboard_protocol::DashboardState;
pub use nsslice_pinned_panel_protocol::PinnedPanelState;
pub use nsslice_shutdown_protocol::ShutdownTrackerState;
pub use nullslop_providers::NO_PROVIDER_ID;
pub use state::{State, StateReadGuard, StateWriteGuard};
pub use tui_signals::TuiSignals;

use nullslop_component_ui::UiRegistry;

/// Standard UI registry type for the nullslop application.
pub type AppUiRegistry = UiRegistry<AppState>;

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
