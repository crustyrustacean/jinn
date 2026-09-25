//! Synchronous mutation projections for shared frontend state.
//!
//! Each projection keeps a mutation closure inside one application-state write
//! lock while exposing the existing operation wrappers for its domain.

use crate::common::state::State;
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use jinn_chat_input_msg::FilePickerState;
use jinn_persona_msg::{PersonaEntry, persona_row};
use jinn_preferences_config::app_state_file::AppStateFile;

/// Narrow write handle to frontend preferences.
pub struct PreferencesOps<'a>(&'a mut FrontendState);

/// Narrow write handle to the persona picker.
pub struct PersonaPickerOps<'a>(&'a mut FrontendState);

/// Narrow write handle to the file-picker state.
pub struct FilePickerOps<'a>(&'a mut FilePickerState);

/// Narrow write handle to persisted application state.
pub struct AppStateOps<'a>(&'a mut AppStateFile);

impl PreferencesOps<'_> {
    /// Mutably access the whole frontend state.
    pub fn frontend(&mut self) -> &mut FrontendState {
        self.0
    }
}

impl PersonaPickerOps<'_> {
    /// Replace persona picker items with the persona render and search hooks.
    pub fn set_items(&mut self, items: Vec<PersonaEntry>) {
        let wrapped = jinn_picker::make_items_with_hooks(
            items,
            jinn_picker::PickerItemHooks::new()
                .row(persona_row)
                .search(|entry: &PersonaEntry| entry.name.clone()),
        );
        self.0.persona_picker_mut().set_items(wrapped);
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
    /// Mutate frontend preferences through [`PreferencesOps`].
    pub fn with_preferences<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut PreferencesOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut PreferencesOps(&mut app.frontend))
    }

    /// Mutate persona picker state through [`PersonaPickerOps`].
    pub fn with_persona_picker<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut PersonaPickerOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut PersonaPickerOps(&mut app.frontend))
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

    /// Mutate provider and endpoint picker state.
    pub fn with_pickers<R, F>(&self, f: F) -> R
    where
        F: FnOnce(&mut crate::feat::ui::picker_states::PickerStates) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut app.frontend.pickers)
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
    use super::*;
    use jinn_selection_widget::TreeItem;

    fn persona(name: &str) -> PersonaEntry {
        PersonaEntry {
            name: name.to_owned(),
            description: "desc".to_owned(),
            is_active: false,
            theme: jinn_theme::default_theme(),
        }
    }

    #[rstest::rstest]
    fn persona_entry_writer_keeps_the_spec_row_renderer() {
        // Given a frontend and one persona entry.
        let mut frontend = FrontendState::default();

        // When writing it through the persona picker projection.
        PersonaPickerOps(&mut frontend).set_items(vec![persona("coder")]);

        // Then the stored item renders through the spec's row hook.
        let row = frontend.persona_picker().items()[0].render_row(false);
        let text: String = row
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert_eq!(text, "  coder  desc");
    }
}
