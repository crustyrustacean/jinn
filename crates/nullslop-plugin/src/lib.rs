//! Plugin system for nullslop.
//!
//! Embeds a Lua VM and provides bindings for emitting commands (`ns.emit`),
//! subscribing to events (`ps.sub`, `ps.pub`, `ps.unsub`), and loading
//! plugins from directories containing `init.lua`.
//!
//! The plugin host does not depend on the TUI crate. It accepts a command
//! sender callback that the TUI wiring provides at startup.

mod bindings;
mod host;
mod loader;
mod preflight;
mod subscriber;

pub use host::{CommandSender, PluginHost, PluginInfo};
pub use subscriber::WelcomeSubscriber;
