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
use crate::feat::dashboard::DashboardState;
use crate::feat::file_lister::FilePickerState;
use crate::feat::persona::PersonaEntry;
use crate::feat::preferences_actor::app_state_file::AppStateFile;
use crate::feat::session::entry_token_cache::EntryTokenCache;
use crate::feat::skills::Skill;
use crate::feat::theme::Theme;
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::ChatEntryId;

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

/// Narrow write-handle to `frontend.dashboard` for the status actor.
pub struct DashboardOps<'a>(&'a mut DashboardState);

/// Narrow write-handle to `frontend.quake_bar` for the quake-bar actor.
/// Exposes the [`QuakeBarLogWrite`] trait.
pub struct QuakeBarOps<'a>(&'a mut crate::feat::quake_bar::state::QuakeBarState);

/// Narrow write-handle to the token-cache `RwLock` for the token-count actor.
/// Exposes the [`TokenCountWrite`] trait.
pub struct TokenCacheOps<'a>(&'a mut parking_lot::RwLock<EntryTokenCache>);

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

/// Append a line to the quake-bar log.
pub trait QuakeBarLogWrite {
    fn push_log(&mut self, text: String);
}

/// Insert / bulk-insert token counts into the entry-token cache.
pub trait TokenCountWrite {
    fn insert(&mut self, id: ChatEntryId, count: u32);
    fn bulk_insert<I: IntoIterator<Item = (ChatEntryId, u32)>>(&mut self, entries: I);
}

// ── Inherent accessors on the Ops newtypes ──────────────────────────────────

impl PreferencesOps<'_> {
    /// Mutable access to the whole frontend (preferences, sidebar, theme, ...).
    pub fn frontend(&mut self) -> &mut FrontendState {
        self.0
    }
}

impl DashboardOps<'_> {
    /// Mutable dashboard, used to call `mark_starting`/`mark_running`/`mark_dead`.
    pub fn dashboard(&mut self) -> &mut DashboardState {
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
    /// Replace the persona picker items.
    pub fn set_items(&mut self, items: Vec<PersonaEntry>) {
        self.0.persona_picker_mut().set_items(items);
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

impl QuakeBarLogWrite for QuakeBarOps<'_> {
    fn push_log(&mut self, text: String) {
        self.0.log.push(text);
    }
}

impl TokenCountWrite for TokenCacheOps<'_> {
    fn insert(&mut self, id: ChatEntryId, count: u32) {
        self.0.write().insert(id, count);
    }
    fn bulk_insert<I: IntoIterator<Item = (ChatEntryId, u32)>>(&mut self, entries: I) {
        self.0.write().bulk_insert(entries);
    }
}

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

    /// Write access to the dashboard grid, scoped via [`DashboardOps`].
    pub fn with_dashboard<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut DashboardOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut DashboardOps(&mut app.frontend.dashboard))
    }

    /// Write access to the quake-bar log, scoped via [`QuakeBarOps`].
    pub fn with_quake_bar<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut QuakeBarOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut QuakeBarOps(&mut app.frontend.quake_bar))
    }

    /// Write access to the entry-token cache, scoped via [`TokenCacheOps`].
    pub fn with_entry_token_cache<R, F>(&self, _cap: &FrontendCap, f: F) -> R
    where
        F: FnOnce(&mut TokenCacheOps<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut TokenCacheOps(
            &mut app.frontend.caches.entry_token_cache,
        ))
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
