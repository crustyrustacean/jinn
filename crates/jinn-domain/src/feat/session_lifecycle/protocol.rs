//! Session lifecycle protocol - commands and events for async execution.

pub mod command;
pub mod event;

pub use command::{
    CancelLifecycleCommand, FinishSessionSetup, PersistSession, RunSessionSetup,
    RunSessionTeardown, SetSessionCwd,
};
pub use event::{SessionCwdChanged, SessionSetupCompleted, SessionTeardownFinished};
