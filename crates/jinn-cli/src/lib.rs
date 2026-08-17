//! jinn-cli: command-line interface definitions for the jinn agent harness.
//!
//! Provides CLI argument parsing via [`Cli`] and the `jinn plugin new`
//! scaffold generator. The actual dispatch and running is handled by the
//! root crate.

pub mod cli;
pub mod plugin_new;

pub use cli::Cli;
