//! Frontend capsule: cap + Ops newtypes + view + extension traits, colocated.
//!
//! Write access to [`FrontendState`] and its sub-fields is gated by an
//! unforgeable ZST token ([`FrontendCap`]). The projection methods
//! ([`State::with_*`]) hand the cap-holder narrow borrowed views scoped to the
//! exact concern they write (preferences, dashboard, quake bar, token cache,
//! skills picker, persona picker, app state).
//!
//! Frontend is owned by IntentHandler (God-mode via [`State::write`]); the
//! actors that also write here (preferences, dashboard, token-count, skills,
//! quake-bar, status, directory-lister) receive [`FrontendCap`] at wiring for
//! their narrow slice.

use std::collections::HashSet;

use crate::common::state::State;
use crate::feat::file_lister::FilePickerState;
use crate::feat::persona::PersonaEntry;
use crate::feat::skills::Skill;
use crate::feat::theme::Theme;
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use jinn_preferences_config::app_state_file::AppStateFile;

// ── The cap ──────────────────────────────────────────────────────────────────

/// Proof of authority to write [`FrontendState`]. Minted only via
/// [`crate::common::tcaps::mint`].
#[derive(Clone, Copy, Debug)]
pub struct FrontendCap(());

impl FrontendCap {
    /// Private constructor scoped to the `tcaps/` subtree.
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}

// ── Per-struct narrow newtypes ───────────────────────────────────────────────

/// Narrow write-handle to all of `FrontendState` for the preferences actor.
/// The tuple field is PRIVATE. The `frontend()` accessor returns the whole
/// `FrontendState` (its public field API is the capsule wall).
pub struct PreferencesOps<'a>(&'a mut FrontendState);

/// Narrow write-handle to the skills picker + preview cache for the skills
/// actor.
pub struct SkillPickerOps<'a>(&'a mut FrontendState);

/// Narrow write-handle to the persona picker for the session-actor context handler.
pub struct PersonaPickerOps<'a>(&'a mut FrontendState);

/// Narrow write-handle to `frontend.file_picker` for the directory-lister actor.
pub struct FilePickerOps<'a>(&'a mut FilePickerState);

/// Narrow write-handle to `frontend.app_state` for the session-actor startup handler.
pub struct AppStateOps<'a>(&'a mut AppStateFile);

// ── Extension traits (the opt-in method menu) ───────────────────────────────

// ── Inherent accessors on the Ops newtypes ──────────────────────────────────

impl PreferencesOps<'_> {
    /// Mutable access to the whole frontend (preferences, sidebar, theme, ...).
    pub fn frontend(&mut self) -> &mut FrontendState {
        self.0
    }
}

impl SkillPickerOps<'_> {
    /// Reload the skills picker entries from the discovered/disabled sets.
    pub fn reload_picker(
        &mut self,
        discovered: &[Skill],
        disabled: &HashSet<String>,
        theme: &Theme,
    ) {
        crate::feat::skills::reload::reload_skill_picker_entries(
            self.0, discovered, disabled, theme,
        );
    }
}

impl PersonaPickerOps<'_> {
    /// Replace the persona picker items, wrapped through the persona
    /// spec's render/search hooks (the storage holds `ProviderPickerEntry`s).
    pub fn set_items(&mut self, items: Vec<PersonaEntry>) {
        let wrapped = jinn_picker::make_items_with_hooks(
            items,
            jinn_picker::PickerItemHooks::new()
                .row(crate::feat::persona::persona_row)
                .search(|entry: &PersonaEntry| entry.name.clone()),
        );
        self.0.persona_picker_mut().set_items(wrapped);
    }
}

impl FilePickerOps<'_> {
    /// Mutable access to the file-picker state.
    pub fn file_picker(&mut self) -> &mut FilePickerState {
        self.0
    }
}

impl AppStateOps<'_> {
    /// Replace the whole app-state file.
    pub fn set(&mut self, app_state: AppStateFile) {
        *self.0 = app_state;
    }
}

// ── Trait impls ─────────────────────────────────────────────────────────────

// ── Projection methods ──────────────────────────────────────────────────────

impl State {
    /// Write access to the whole frontend (preferences/sidebar/theme), scoped via
    /// [`PreferencesOps`].
    pub fn with_preferences<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut PreferencesOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut PreferencesOps(&mut app.frontend))
    }

    /// Write access to the skills picker, scoped via [`SkillPickerOps`].
    pub fn with_skills_frontend<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut SkillPickerOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut SkillPickerOps(&mut app.frontend))
    }

    /// Write access to the persona picker, scoped via [`PersonaPickerOps`].
    pub fn with_persona_picker<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut PersonaPickerOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut PersonaPickerOps(&mut app.frontend))
    }

    /// Write access to the app-state file, scoped via [`AppStateOps`].
    pub fn with_frontend_app_state<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut AppStateOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut AppStateOps(&mut app.frontend.app_state))
    }

    /// Write access to the provider/endpoint picker surfaces, scoped via
    /// [`PickerStatesWrite`]. The provider-selection slice's actor fills
    /// these at load time (the render/navigation surface the picker host
    /// lends from `&AppState`; the source data lives on the provider cell).
    pub fn with_pickers<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut crate::feat::ui::picker_states::PickerStates) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut app.frontend.pickers)
    }

    /// Write access to `frontend.file_picker`, scoped via [`FilePickerOps`].
    pub fn with_file_picker<R, F>(&self, _cap: &FrontendCap, f: F) -> R
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
    use crate::feat::persona::PersonaEntry;
    use jinn_selection_widget::TreeItem;

    fn persona(name: &str) -> PersonaEntry {
        PersonaEntry {
            name: name.to_owned(),
            description: "desc".to_owned(),
            is_active: false,
            theme: crate::feat::theme::default_theme(),
        }
    }

    #[rstest::rstest]
    fn persona_entry_writer_keeps_the_spec_row_renderer() {
        // Given a frontend and one persona entry.
        let mut frontend = FrontendState::default();

        // When writing it through the persona picker cap.
        PersonaPickerOps(&mut frontend).set_items(vec![persona("coder")]);

        // Then the stored item renders through the spec's row hook: the
        // active marker, name, and description — not the bare search label.
        let row = frontend.persona_picker().items()[0].render_row(false);
        let text: String = row.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "  coder  desc");
    }
}
