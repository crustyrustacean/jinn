//! Plugin system for nullslop.
//!
//! Each plugin runs in its own sandboxed Lua VM. The [`PluginRegistry`]
//! owns all VMs and provides a centralized API for dispatching events
//! to plugins and collecting hook results.
//!
//! The plugin crate defines interfaces ([`TranslatorFn`], [`PluginRegistry`]).
//! The wiring layer (in the main binary) provides the concrete command
//! translation mapping.

mod bindings;
mod hooks;
mod host;
mod loader;
mod preflight;
mod registry;
mod subscriber;
mod translator;

pub use host::PluginHost;
pub use registry::{CommandSender, PluginError, PluginInfo, PluginRegistry};
pub use subscriber::WelcomeSubscriber;
pub use translator::TranslatorFn;
