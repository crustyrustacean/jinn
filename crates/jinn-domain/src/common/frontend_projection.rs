//! Synchronous mutation projections for shared frontend state.
//!
//! Each projection keeps a mutation closure inside one application-state write
//! lock while exposing the existing operation wrappers for its domain.

use crate::common::state::State;
use crate::feat::ui::frontend_state::FrontendState;
use jinn_chat_input_msg::FilePickerState;
use jinn_preferences_config::app_state_file::AppStateFile;

/// Narrow write handle to the frontend's persisted view state.
///
/// Named for what it carries, not for a field: `state.toml` lands here
/// alongside theme, sidebar width, and the picker caches. Config does
/// not — it is read through the configuration layer, never cached here.
pub struct FrontendStateOps<'a>(&'a mut FrontendState);

/// Narrow write handle to the file-picker state.
pub struct FilePickerOps<'a>(&'a mut FilePickerState);

/// Narrow write handle to persisted application state.
pub struct AppStateOps<'a>(&'a mut AppStateFile);

impl FrontendStateOps<'_> {
    /// Mutably access the whole frontend state.
    pub fn frontend(&mut self) -> &mut FrontendState {
        self.0
    }
}

impl FilePickerOps<'_> {
    /// Mutably access the file-picker state.
    pub fn file_picker(&mut self) -> &mut FilePickerState {
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

    /// Mutate file-picker state through [`FilePickerOps`].
    pub fn with_file_picker<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut FilePickerOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut FilePickerOps(&mut app.frontend.file_picker))
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
