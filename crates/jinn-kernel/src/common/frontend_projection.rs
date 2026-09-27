//! Synchronous mutation projections for shared frontend state.
//!
//! Each projection keeps a mutation closure inside one application-state write
//! lock while exposing the existing operation wrappers for its domain.
//!
//! Cell-backed state is the exception: the `@path` file popup resolves
//! through its own cell, so reaching it never takes the application-state
//! lock at all.

use crate::common::state::State;
use crate::state::frontend_state::FrontendState;
use jinn_preferences_config::app_state_file::AppStateFile;

/// Narrow write handle to the frontend's persisted view state.
///
/// Named for what it carries, not for a field: `state.toml` lands here
/// alongside theme, sidebar width, and the picker caches. Config does
/// not — it is read through the configuration layer, never cached here.
pub struct FrontendStateOps<'a>(&'a mut FrontendState);

/// Narrow write handle to persisted application state.
pub struct AppStateOps<'a>(&'a mut AppStateFile);

impl FrontendStateOps<'_> {
    /// Mutably access the whole frontend state.
    pub fn frontend(&mut self) -> &mut FrontendState {
        self.0
    }
}

impl AppStateOps<'_> {
    /// Replace the whole persisted application state.
    pub fn set(&mut self, app_state: AppStateFile) {
        *self.0 = app_state;
    }
}

impl State {
    /// Mutate the frontend's persisted view state through
    /// [`FrontendStateOps`].
    pub fn with_frontend_state<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut FrontendStateOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut FrontendStateOps(&mut app.frontend))
    }

    /// Mutate persisted application state through [`AppStateOps`].
    pub fn with_frontend_app_state<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut AppStateOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut AppStateOps(&mut app.frontend.app_state))
    }

    /// Mutate the `@path` file popup's state, returning `f`'s result.
    ///
    /// The popup lives in a cell, so this resolves it through the
    /// frontend's attached slice registry rather than through the
    /// application-state write lock — reaching the payload no longer
    /// contends with the render pass for `AppState`. `None` means the
    /// chat-input slice was never activated.
    pub fn with_file_picker<R, F>(&self, f: F) -> Option<R>
    where
        R: Sized,
        F: FnOnce(&mut jinn_chat_input_msg::FilePickerState) -> R,
    {
        let guard = self.read();
        guard.frontend.update_file_picker(f)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test module, panics are acceptable"
    )]
}
