//! Standardized render context for the TUI render path.
//!
//! [`RenderCtx`] wraps a shared reference to [`AppState`] and is threaded
//! through every render function. It provides a single, extensible context
//! type that can grow to hold plugin registries, command sinks, or other
//! capabilities without changing function signatures.

use crate::common::app_state::AppState;
use crate::feat::plugin_dispatch::PluginSyncHooks;

/// Render context passed to every render function.
///
/// Contains read-only access to application state. Constructed once per
/// frame in the top-level `render()` function and passed through the
/// entire render tree.
///
/// The optional `plugins` handle lets individual render call sites query
/// sync plugin hooks (e.g. `on_chat_input_badges_render`) directly, picking
/// the hook and building their own ctx at the call site.
pub struct RenderCtx<'a> {
    /// Read-only application state.
    pub state: &'a AppState,
    /// Optional sync plugin-hooks handle (render-thread direct path).
    pub plugins: Option<&'a dyn PluginSyncHooks>,
}

impl<'a> RenderCtx<'a> {
    /// Creates a new render context wrapping the given state reference.
    ///
    /// Plugins are available via [`RenderCtx::with_plugins`].
    pub fn new(state: &'a AppState) -> Self {
        Self {
            state,
            plugins: None,
        }
    }

    /// Attaches the sync plugin-hooks handle so render call sites can query
    /// hooks (e.g. badge directives) directly.
    #[must_use]
    pub fn with_plugins(mut self, plugins: &'a dyn PluginSyncHooks) -> Self {
        self.plugins = Some(plugins);
        self
    }
}
