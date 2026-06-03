//! Standardized render context for the TUI render path.
//!
//! [`RenderCtx`] wraps a shared reference to [`AppState`] and is threaded
//! through every render function. It provides a single, extensible context
//! type that can grow to hold plugin registries, command sinks, or other
//! capabilities without changing function signatures.

use crate::common::app_state::AppState;

/// Render context passed to every render function.
///
/// Contains read-only access to application state. Constructed once per
/// frame in the top-level `render()` function and passed through the
/// entire render tree.
pub struct RenderCtx<'a> {
    /// Read-only application state.
    pub state: &'a AppState,
}

impl<'a> RenderCtx<'a> {
    /// Creates a new render context wrapping the given state reference.
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}
