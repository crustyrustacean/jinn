//! Session-lifecycle crossing contracts and shared leaf vocabulary.
//!
//! This crate is the low-level export surface of the session-lifecycle family:
//! setup/teardown messages cross the bus here, while the command-template,
//! builtin-registry, and picker-row types live here because kernel frontend
//! code still consumes them. The lifecycle slice itself depends on this crate;
//! the kernel depends on this crate. Neither direction forms a cycle.

pub mod builtin;
pub mod command;
pub mod command_template;
pub mod event;
pub mod picker_entry;

pub use builtin::{BuiltinHandler, BuiltinHandlerError, BuiltinRegistry};
pub use command::{
    CancelLifecycleCommand, FinishSessionSetup, FinishSessionTeardown, PersistSession,
    RunSessionSetup, RunSessionTeardown, SetSessionCwd, TeardownFollowUp,
};
pub use command_template::CommandTemplate;
pub use event::{SessionCreated, SessionCwdChanged, SessionSetupCompleted, SessionTeardownFinished};
pub use picker_entry::{SessionLifecycleEntry, lifecycle_row};
