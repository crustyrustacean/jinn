//! Frontend / UI state - owned by the IntentHandler (main thread).

use parking_lot::RwLock;

use crate::common::focus::{FocusScope, ScopeStack};
use crate::common::tui_signals::TuiSignals;
use crate::feat::preferences_actor::UserPreferences;
use crate::feat::cwd_input::state::CwdInputState;
use crate::feat::rename_session_input::state::RenameSessionInputState;

use crate::feat::session_lifecycle::arg_input_state::ArgInputState;
use crate::feat::theme::Theme;
use crate::feat::ui::picker_states::PickerStates;
pub use crate::feat::ui::sidebar::persona_section::PersonaSectionState;
pub use crate::feat::ui::sidebar::pins::state::PinsState;
pub use crate::feat::ui::sidebar::sessions::SessionsSectionState;
use crate::feat::ui::sidebar::state::SidebarState;
pub use crate::feat::ui::sidebar::task_list_section::TaskListSectionState;

/// Theme-sensitive caches owned by the frontend.
///
/// All caches that store pre-rendered styled data (which embeds theme colors)
/// live here so they can be invalidated in one call when the theme changes.
///
/// Each cache is wrapped in a `RwLock` so render code can borrow mutably
/// while holding shared references to the rest of `AppState`.
#[derive(Debug, Default)]
pub struct FrontendCaches {
    /// Cached wrapped line counts and rendered lines per chat entry.
    pub entry_line_cache: RwLock<crate::feat::ui::chat_log::line_count_cache::EntryLineCache>,
    /// Cached token counts per chat entry (tiktoken-based).
    /// Populated by the token count actor, read by the minimap render pipeline.
    pub entry_token_cache: RwLock<crate::feat::session::entry_token_cache::EntryTokenCache>,
    /// Cached rendered lines for skill-preview popups.
    pub skill_preview_cache: RwLock<crate::feat::skills::skill_preview_cache::SkillPreviewCache>,
    /// Cached rendered lines for session preview popups.
    pub session_preview_cache:
        RwLock<crate::feat::ui::sidebar::sessions::preview::SessionPreviewCache>,
}

impl FrontendCaches {
    /// Invalidate all caches. Called when the active theme changes.
    pub fn invalidate_all(&self) {
        self.entry_line_cache.write().clear();
        self.session_preview_cache.write().clear();
        self.skill_preview_cache.write().clear();
    }
}

/// Frontend / UI state - owned by the IntentHandler (main thread).
///
/// Written to by `IntentHandler` and various UI elements (read-only).
/// Actors should NOT write to these fields - they are for the frontend only.
#[derive(Debug)]
pub struct FrontendState {
    /// Set to `true` when the user has requested to quit.
    /// OWNER: IntentHandler (Quit intent),
    ///        shutdown-tracker (ProceedWithShutdown command).
    pub should_quit: bool,

    /// Pins sidebar section state - selection index within the pinned entries list.
    /// OWNER: IntentHandler (pins navigation).
    pub pins: PinsState,

    /// Sidebar state - focus tracking.
    /// OWNER: IntentHandler (sidebar focus/leave).
    pub sidebar: SidebarState,

    /// Persona sidebar section state - cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub persona_section: PersonaSectionState,

    /// Sessions sidebar section state - cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub sessions_section: SessionsSectionState,

    /// Task list sidebar section state - phase cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub task_list_section: TaskListSectionState,
    /// Signals from the IntentHandler for the outer platform layer.
    /// OWNER: IntentHandler (cleared and set each handle() call).
    pub tui_signals: TuiSignals,

    /// Cached copy of user preferences from `jinn.toml`.
    /// Updated exclusively by `PreferencesStateSyncActor` on `PreferencesUpdated` events.
    /// This is a cache - the file is the authoritative source.
    pub preferences: UserPreferences,

    /// Focus scope stack - single source of truth for what the user is focused on.
    /// OWNER: IntentHandler (push/pop on scope transitions).
    pub scope_stack: ScopeStack,

    /// The current resolved theme (colors for the render pipeline).
    /// OWNER: IntentHandler (theme picker preview), PreferencesStateSyncActor (on prefs change).
    pub theme: Theme,

    /// Theme-sensitive caches. Invalidated when `theme` changes.
    /// OWNER: IntentHandler (cleared on theme change).
    pub caches: FrontendCaches,

    /// Whether the "Press ESC again to cancel" prompt is showing.
    /// OWNER: IntentHandler (set on first ESC in Normal/Sidebar with active stream,
    ///         consumed on second ESC or dismissed on any other key).
    pub cancel_stream_prompt: bool,

    /// Whether the audit popup is shown for the currently selected chat entry.
    /// OWNER: IntentHandler (ToggleAuditPopup intent).
    /// Global toggle (not per-session); not persisted across process restarts.
    pub audit_popup_visible: bool,

    /// Whether the "Press x again to confirm closure" prompt is showing.
    /// OWNER: IntentHandler (set on first SidebarSessionClose, consumed on second
    ///         SidebarSessionClose or dismissed on any other key).
    pub close_session_prompt: bool,

    /// All picker state - grouped for independent evolution.
    /// Use [`PickerExt`](super::picker_states::PickerExt) to access picker fields.
    pub pickers: PickerStates,

    /// Path to the themes directory (`~/.config/jinn/themes/`).
    /// Set once during init from `AppPaths`. Used by the theme picker to discover themes.
    /// OWNER: Init code (set once at startup).
    pub themes_dir: std::path::PathBuf,

    /// Path to the system themes directory (`/usr/share/jinn/themes/`).
    /// Set once during init from `AppPaths`. Used as fallback for theme discovery.
    /// OWNER: Init code (set once at startup).
    pub system_themes_dir: std::path::PathBuf,

    /// Arg input popup state - active when `FocusScope::ArgInput` is on the scope stack.
    /// OWNER: IntentHandler (arg input editing, confirmation).
    pub arg_input: ArgInputState,

    /// Rename session input popup state - active when `FocusScope::RenameSessionInput` is on the scope stack.
    /// OWNER: IntentHandler (rename input editing, confirmation).
    pub rename_session_input: RenameSessionInputState,

    /// Cwd input popup state - active when `FocusScope::CwdInput` is on the scope stack.
    /// OWNER: IntentHandler (cwd input editing, confirmation).
    pub cwd_input: CwdInputState,

    pub sidebar_width: u16,
}

impl Default for FrontendState {
    fn default() -> Self {
        let mut scope_stack = ScopeStack::default();
        scope_stack.push(FocusScope::Input);
        Self {
            should_quit: false,
            pins: PinsState::default(),
            sidebar: SidebarState::default(),
            persona_section: PersonaSectionState::default(),
            sessions_section: SessionsSectionState::default(),
            task_list_section: TaskListSectionState::default(),
            tui_signals: TuiSignals::new(),
            preferences: UserPreferences::default(),
            scope_stack,
            theme: crate::feat::theme::default_theme(),
            caches: FrontendCaches::default(),
            cancel_stream_prompt: false,
            audit_popup_visible: false,
            close_session_prompt: false,
            pickers: PickerStates::default(),
            themes_dir: std::path::PathBuf::new(),
            system_themes_dir: std::path::PathBuf::new(),
            arg_input: ArgInputState::default(),
            rename_session_input: RenameSessionInputState::default(),
            cwd_input: CwdInputState::default(),

            sidebar_width: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;
    use jinn_selection_widget::PreviewCache;
    use ratatui::text::Line;

    /// `invalidate_all` (called on theme change) must clear the skill preview cache
    /// so stale theme-colored lines are never displayed after a theme switch.
    #[test]
    fn invalidate_all_clears_skill_preview_cache() {
        // Given a populated skill preview cache.
        let caches = FrontendCaches::default();
        caches
            .skill_preview_cache
            .write()
            .insert("web-coder".to_owned(), 80, vec![Line::raw("old-theme")]);
        assert_eq!(caches.skill_preview_cache.read().len(), 1);

        // When the theme changes and all caches are invalidated.
        caches.invalidate_all();

        // Then the skill preview cache is empty (the AC under test).
        assert!(
            caches.skill_preview_cache.read().is_empty(),
            "theme change must clear skill preview cache via invalidate_all"
        );
    }
}
