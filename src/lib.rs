//! jinn: a TUI agent harness with a component/actor system.

pub mod actor_wiring;
pub mod app;
pub mod headless;
pub mod plugin_actor;
pub mod plugin_wiring;
pub mod runner;
pub mod tracing;

pub use app::{App, AppError};
pub use headless::HeadlessApp;
pub use runner::Runner;
