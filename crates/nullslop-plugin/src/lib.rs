//! Plugin runtime — rhai-based scripting for nullslop.
//!
//! Provides the core types and runtime for loading and executing rhai plugins.
//! The [`PluginRuntime`] wraps a per-plugin rhai `Engine` and [`Scope`],
//! and the [`loader`] module discovers plugins on disk.

pub mod app_info;
pub mod command_allowlist;
pub mod error;
pub mod loader;
pub mod plugin_id;
pub mod runtime;

pub use error::PluginError;
pub use plugin_id::PluginId;
pub use runtime::PluginRuntime;
