//! Session-lifecycle crossing contracts and shared leaf vocabulary.
//!
//! This crate is the low-level export surface of the session-lifecycle family:
//! setup/teardown messages cross the bus here, while the command-template,
//! builtin-registry, and picker-row types live here. The session-lifecycle
//! picker's identity and per-open state live here too, so the project picker
//! can reach this picker's scope without depending on the implementation
//! crate. The lifecycle slice itself depends on this crate;
//! the kernel depends on this crate. Neither direction forms a cycle.

pub mod arg_input;
pub mod builtin;
pub mod command;
pub mod command_template;
pub mod event;
pub mod lifecycle_script_state;
pub mod picker_entry;
pub mod picker_scope;
pub mod picker_state;

pub use arg_input::{ArgInputState, arg_input_scope, arg_input_slot};
pub use builtin::{BuiltinHandler, BuiltinHandlerError, BuiltinRegistry};
pub use command::{
    CancelLifecycleCommand, CloseSession, FinishSessionSetup, FinishSessionTeardown,
    RunSessionSetup, RunSessionTeardown, SetSessionCwd, TeardownFollowUp, TeardownSessionTree,
};
pub use command_template::CommandTemplate;
pub use event::{SessionCreated, SessionCwdChanged};
pub use jinn_session_msg::{SessionSetupCompleted, SessionTeardownFinished};
pub use lifecycle_script_state::LifecycleScriptState;
pub use picker_entry::{SessionLifecycleEntry, lifecycle_row};
pub use picker_scope::session_lifecycle_picker_scope;
pub use picker_state::{
    RESULTS_VIEWPORT_FALLBACK, SessionLifecyclePickerState, session_lifecycle_picker_slot,
};

/// The system entry shown while a session setup command is running.
///
/// The kernel IntentHandler publishes this entry before the lifecycle actor
/// takes over, so the message builder belongs to the shared family crate.
#[must_use]
pub fn setup_running_msg() -> jinn_core_types::ChatEntry {
    jinn_core_types::ChatEntry::system("⚙️ Running setup script...")
}
