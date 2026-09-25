//! Session lifecycle protocol - commands and events for async execution.

pub mod command;
pub mod event;

pub use command::{
    CancelLifecycleCommand, FinishSessionSetup, RunSessionSetup, RunSessionTeardown,
    SetSessionCwd, TeardownFollowUp,
};
pub use event::{SessionCwdChanged, SessionSetupCompleted, SessionTeardownFinished};
