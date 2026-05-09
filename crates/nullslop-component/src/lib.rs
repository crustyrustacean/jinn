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
//! - [`AppBus`] — the standard message bus for the application.
//! - [`AppUiRegistry`] — the standard UI element registry.

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
pub mod open_picker_handler;
pub mod picker_highlight;
pub mod pinned_panel;
pub mod prompt_template;
pub mod provider;
pub mod provider_picker;
pub mod session_picker;
pub mod shutdown_tracker;
pub mod status_bar;
pub mod tab_nav;
pub mod tui_signals;

pub use app_state::AppState;
pub use nullslop_providers::NO_PROVIDER_ID;
pub use tui_signals::TuiSignals;
pub use chat_input_box::ChatInputBoxState;
pub use chat_session::ChatSessionState;
pub use dashboard::DashboardState;
pub use shutdown_tracker::ShutdownTrackerState;

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

use nullslop_component_core::Bus;
use nullslop_component_ui::UiRegistry;
use nullslop_services::Services;

/// Standard bus type for the nullslop application.
pub type AppBus = Bus<AppState, Services>;

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

/// Register all built-in components with the bus and UI registry.
///
/// Called once during application startup.
pub fn register_all(bus: &mut AppBus, registry: &mut AppUiRegistry) {
    app_quit::register(bus, registry);
    context_pin::register(bus, registry);
    chat_entry_selection::register(bus, registry);
    shutdown_tracker::register(bus, registry);
    chat_input_box::register(bus, registry);
    chat_log::register(bus, registry);
    char_counter::register(bus, registry);
    dashboard::register(bus, registry);
    tab_nav::register(bus, registry);
    provider::register(bus, registry);
    provider_picker::register(bus, registry);
    session_picker::register(bus, registry);
    pinned_panel::register(bus, registry);
    status_bar::register(bus, registry);
    open_picker_handler::OpenPickerHandler.register(bus);
    prompt_template::rescan_handler::RescanHandler.register(bus);
}

/// Register only TUI elements (no bus handlers).
///
/// Use when bus handlers have already been registered elsewhere
/// (e.g., by [`register_all`] during core creation) and only
/// the UI element registry needs to be populated.
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

// Phase 5: macro_tests disabled — tests reference removed Command variants (Quit, InsertChar).
// Will be rewritten when new UI command types are added.
// #[cfg(test)]
// mod macro_tests { ... }
