//! Example workflow definitions.
//!
//! Contains built-in workflow graph builders that can be registered at startup
//! and triggered via `/workflow`.

pub mod add_numbers;

pub use add_numbers::register;
