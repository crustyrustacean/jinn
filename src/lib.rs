//! jinn: a TUI agent harness with a component/actor system.

pub mod actor_wiring;
pub mod app;
#[cfg(debug_assertions)]
pub mod headless;
#[cfg(feature = "disabled-during-migration")]
pub mod plugin_wiring;
pub mod runner;
pub mod tracing;

pub use app::{App, AppError};
#[cfg(debug_assertions)]
pub use headless::HeadlessApp;
pub use runner::Runner;
