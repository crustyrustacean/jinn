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
//!
//! # Phase 5+6 status
//!
//! All handler code has been removed. Components contain only state structs and
//! UI elements. Domain logic will be re-implemented as Coordinator/Projector
//! actors in Phase 7.

pub mod app_quit;
pub mod app_state;
pub mod char_counter;
pub mod chat_entry_selection;
pub mod chat_input_box;
pub mod chat_log;
pub mod chat_session;
pub mod context_pin;
pub mod context_strategy_picker;
pub mod dashboard;
pub mod keymap_picker;
pub mod picker_highlight;
pub mod pinned_panel;
pub mod prompt_template;
pub mod provider;
pub mod provider_picker;
pub mod session_picker;
pub mod shutdown_tracker;
pub mod state;
pub mod status_bar;
pub mod tab_nav;
pub mod tui_signals;

pub use app_state::{
    AppState, ContextAssemblyState, FrontendState, ProviderState, SessionState,
    ShutdownCoordinatorState,
};
pub use chat_input_box::ChatInputBoxState;
pub use chat_session::ChatSessionState;
pub use dashboard::DashboardState;
pub use nullslop_providers::NO_PROVIDER_ID;
pub use shutdown_tracker::ShutdownTrackerState;
pub use state::{State, StateReadGuard, StateWriteGuard};
pub use tui_signals::TuiSignals;

use nullslop_component_ui::UiRegistry;

/// Standard UI registry type for the nullslop application.
pub type AppUiRegistry = UiRegistry<AppState>;

use ratatui::style::{Color, Modifier, Style};

/// Highlight style for fuzzy-matched characters in picker rows.
///
/// Shared across all picker entry types so the look is consistent.
/// Dark gray background with underline; foreground is inherited from the base style.
pub const PICKER_HIGHLIGHT_STYLE: Style = Style::new()
    .bg(Color::DarkGray)
    .add_modifier(Modifier::UNDERLINED);

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
pub fn register_tui_elements(registry: &mut AppUiRegistry) {
    registry.register(Box::new(chat_input_box::ChatInputBoxElement));
    registry.register(Box::new(chat_log::ChatLogElement));
    registry.register(Box::new(char_counter::CharCounterElement));
    registry.register(Box::new(dashboard::DashboardElement));
    registry.register(Box::new(pinned_panel::PinnedPanelElement));
    registry.register(Box::new(
        provider::indicator::StreamingIndicatorElement::new(),
    ));
    registry.register(Box::new(provider::queue_element::QueueDisplayElement));
    registry.register(Box::new(status_bar::StatusBarElement));
}

/// Test utilities shared across the crate.
///
/// Only available in `#[cfg(test)]` builds.
#[cfg(test)]
pub(crate) mod test_utils {
    use nullslop_services::Services;

    /// Create a [`nullslop_services::Services`] with fake implementations for tests.
    pub fn test_services() -> Services {
        Services::new()
    }
}
