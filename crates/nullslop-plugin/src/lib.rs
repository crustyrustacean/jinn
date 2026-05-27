//! Plugin system for nullslop.
//!
//! Each plugin runs in its own sandboxed Lua VM. The [`PluginRegistry`]
//! owns all VMs and provides a centralized API for dispatching events
//! to plugins and collecting hook results.
//!
//! The plugin crate defines interfaces ([`TranslatorFn`], [`PluginRegistry`]).
//! The wiring layer (in the main binary) provides the concrete command
//! translation mapping.

pub(crate) mod bindings;
pub mod ctx;
pub mod hooks;
pub(crate) mod loader;
pub(crate) mod registry;
pub(crate) mod translator;

pub use registry::{CommandSender, PluginError, PluginInfo, PluginRegistry};
pub use translator::TranslatorFn;

// ── Free-function API ──────────────────────��──────────────────────────────
//
// These are the primary interface for host code. Using free functions
// (instead of methods) makes every plugin interaction grep-friendly:
//   grep -rn 'plugin::emit\|plugin::for_hook' src/

/// Fire-and-forget event dispatch to all plugin VMs.
///
/// Serializes `ctx` to JSON, converts to Lua values, and calls every
/// `ps.sub` callback registered for `event_name`. Individual callback
/// errors are logged as warnings.
///
/// # Example
///
/// ```ignore
/// plugin::emit(hooks::APP_STARTED, &registry, &AppStartedCtx { session_id });
/// ```
pub fn emit(event_name: &str, registry: &PluginRegistry, ctx: &impl serde::Serialize) {
    registry.emit(event_name, ctx);
}

/// Data-returning hook call to all plugin VMs.
///
/// Serializes `ctx`, calls every `ps.hook` callback registered for
/// `hook_name`, and deserializes each return value into `T`. Individual
/// failures are logged as warnings and excluded from results.
///
/// # Example
///
/// ```ignore
/// let items: Vec<MyItem> = plugin::for_hook("render_sidebar", &registry, &ctx);
/// ```
pub fn for_hook<T>(
    hook_name: &str,
    registry: &PluginRegistry,
    ctx: &impl serde::Serialize,
) -> Vec<T>
where
    T: serde::de::DeserializeOwned,
{
    registry.for_hook(hook_name, ctx)
}
