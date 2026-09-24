//! The session-lifecycle slice — setup, teardown, close, and cwd changes.
//!
//! This slice owns the lifecycle actor: it runs a session's setup command,
//! its teardown command, cancellation, close-with-teardown, and working
//! directory changes. The crossing contracts and the leaf vocabulary the
//! kernel frontend still consumes live in `jinn-session-lifecycle-msg`.
//! The kernel frontend handlers (`intent.rs`, `render.rs`) remain in
//! `jinn-domain`, as does the session-state vocabulary they mutate.

pub mod command_runner;

pub use command_runner::{
    LifecycleCancelHandle, LifecycleCommandError, spawn_setup_command, spawn_teardown_command,
};
