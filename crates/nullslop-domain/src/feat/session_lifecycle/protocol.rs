//! Session lifecycle protocol — commands and events for async execution.

pub mod command;
pub mod event;

pub use command::{RunSessionSetup, RunSessionTeardown};
pub use event::{SessionSetupCompleted, SessionTeardownCompleted};
