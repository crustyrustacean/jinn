//! Session lifecycle management — setup/teardown command templates for sessions.
//!
//! Provides [`CommandTemplate`] for parsing and rendering shell command strings
//! that contain positional parameters (`$1`, `$2`, `$@`). Used by session
//! lifecycle recipes to bootstrap and tear down working directories.

pub mod arg_input_state;
pub mod builtin;
pub mod command_runner;
pub mod command_template;
pub mod intent;
pub mod picker_entry;
pub mod protocol;
pub mod render;
