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
pub mod lifecycle_script_state;
pub mod picker_entry;

pub use builtin::{BuiltinHandler, BuiltinHandlerError, BuiltinRegistry};
pub use command::{
    CancelLifecycleCommand, CloseSession, FinishSessionSetup, FinishSessionTeardown,
    PersistSession, RunSessionSetup, RunSessionTeardown, SetSessionCwd, TeardownFollowUp,
    TeardownSessionTree,
};
pub use command_template::CommandTemplate;
pub use event::{
    SessionCreated, SessionCwdChanged, SessionSetupCompleted, SessionTeardownFinished,
};
pub use lifecycle_script_state::LifecycleScriptState;
pub use picker_entry::{SessionLifecycleEntry, lifecycle_row};

/// The system entry shown while a session setup command is running.
///
/// The kernel IntentHandler publishes this entry before the lifecycle actor
/// takes over, so the message builder belongs to the shared family crate.
#[must_use]
pub fn setup_running_msg() -> jinn_core_types::ChatEntry {
    jinn_core_types::ChatEntry::system("⚙️ Running setup script...")
}
