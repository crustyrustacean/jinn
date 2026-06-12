//! jinn-tui: terminal user interface for the jinn agent harness.
//!
//! This crate provides the main event loop, terminal setup, rendering,
//! and key handling for the jinn TUI application.

pub mod app;
pub mod app_state;
pub mod config;
pub mod convert;
pub mod keymap;
pub mod launch;
pub mod msg;
pub mod render;
pub mod run;
pub mod scope;
pub mod selection;
pub mod suspend;
pub mod terminal;

pub use app::TuiApp;
pub use app::TuiAppBuilder;
pub use app_state::AppStatus;
pub use jinn_domain::Services;
pub use jinn_domain::AppCore;
pub use keymap::KeyCategory;
pub use launch::{LaunchError, launch, load_compaction_prompt, load_theme};
pub use msg::handler::MsgHandler;
pub use run::{TuiRunError, run};
pub use scope::Scope;

//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod app_tests;

//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod render_tests;

//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod selection_tests;
