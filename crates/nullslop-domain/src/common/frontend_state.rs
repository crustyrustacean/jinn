//! Frontend / UI state — owned by the IntentHandler (main thread).

use std::collections::HashSet;

use parking_lot::RwLock;

use crate::common::focus::{FocusScope, ScopeStack};
use crate::common::tui_signals::TuiSignals;
use crate::feat::persona::PersonaEntry;
use crate::feat::preferences_actor::UserPreferences;
use crate::feat::rename_session_input::state::RenameSessionInputState;
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::session_lifecycle::arg_input_state::ArgInputState;
use crate::feat::session_lifecycle::picker_entry::SessionLifecycleEntry;
use crate::feat::skills::skill_entry::SkillEntry;
use crate::feat::theme::Theme;
use crate::feat::theme::ThemeEntry;
use crate::feat::tools_actor::tool_entry::ToolEntry;
use crate::feat::workflow::picker_entry::WorkflowPickerEntry;
use crate::feat::workflow::workflow_ui_state::WorkflowUiState;
use crate::feat::judge::JudgePickerEntry;
pub use crate::feat::ui::sidebar::persona_section::PersonaSectionState;
pub use crate::feat::ui::sidebar::pins::state::PinsState;
pub use crate::feat::ui::sidebar::sessions::SessionsSectionState;
use crate::feat::ui::sidebar::state::SidebarState;
use crate::protocol::PickerEntry;
use crate::protocol::tab::ActiveTab;

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
    /// Cached rendered lines for session preview popups.
    pub session_preview_cache: RwLock<crate::feat::ui::sidebar::sessions::preview::SessionPreviewCache>,
    /// Cached tiktoken-based token counts per chat entry.
    ///
    /// Populated by the token count actor, read by the minimap render pipeline.
    /// Not invalidated on theme change — token counts are theme-independent.
    pub entry_token_cache: RwLock<crate::feat::session::entry_token_cache::EntryTokenCache>,
}

impl FrontendCaches {
    /// Invalidate all caches. Called when the active theme changes.
    pub fn invalidate_all(&self) {
        self.entry_line_cache.write().clear();
        self.session_preview_cache.write().clear();
        // Note: entry_token_cache is NOT cleared here. Token counts are
        // theme-independent and don't need re-computation on theme change.
    }
}

/// Frontend / UI state — owned by the IntentHandler (main thread).
///
/// Written to by `IntentHandler` and various UI elements (read-only).
/// Actors should NOT write to these fields — they are for the frontend only.
#[derive(Debug)]
pub struct FrontendState {
    /// Set to `true` when the user has requested to quit.
    /// OWNER: IntentHandler (Quit intent),
    ///        shutdown-tracker (ProceedWithShutdown command).
    pub should_quit: bool,

    /// Pins sidebar section state — selection index within the pinned entries list.
    /// OWNER: IntentHandler (pins navigation).
    pub pins: PinsState,

    /// Sidebar state — focus tracking.
    /// OWNER: IntentHandler (sidebar focus/leave).
    pub sidebar: SidebarState,

    /// Persona sidebar section state — cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub persona_section: PersonaSectionState,

    /// Sessions sidebar section state — cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub sessions_section: SessionsSectionState,

    /// Signals from the IntentHandler for the outer platform layer.
    /// OWNER: IntentHandler (cleared and set each handle() call).
    pub tui_signals: TuiSignals,

    /// Cached copy of user preferences from `nullslop.toml`.
    /// Updated exclusively by `PreferencesStateSyncActor` on `PreferencesUpdated` events.
    /// This is a cache — the file is the authoritative source.
    pub preferences: UserPreferences,

    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker: nullslop_selection_widget::TreePickerState<SessionTreeEntry>,

    /// Persona picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (persona picker navigation).
    pub persona_picker: nullslop_selection_widget::SelectionState<PersonaEntry>,

    /// Focus scope stack — single source of truth for what the user is focused on.
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

    /// Whether the "Press x again to confirm closure" prompt is showing.
    /// OWNER: IntentHandler (set on first SidebarSessionClose, consumed on second
    ///         SidebarSessionClose or dismissed on any other key).
    pub close_session_prompt: bool,

    /// Theme picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (theme picker navigation).
    pub theme_picker: nullslop_selection_widget::SelectionState<ThemeEntry>,

    /// Saved theme before preview — restored on ESC.
    /// OWNER: IntentHandler (set on theme picker open, consumed on confirm/cancel).
    pub theme_preview_original: Option<Theme>,

    /// Tool picker state — shows all registered tools with toggle state.
    /// OWNER: IntentHandler (populated on tool picker open).
    pub tool_picker: nullslop_selection_widget::SelectionState<ToolEntry>,

    /// Snapshot of disabled tools before picker opens — restored on ESC.
    /// OWNER: IntentHandler (set on tool picker open, consumed on confirm/cancel).
    pub tool_picker_snapshot: Option<HashSet<String>>,

    /// Skill picker state — shows all discovered skills with toggle state.
    /// OWNER: IntentHandler (populated on skill picker open).
    pub skill_picker: nullslop_selection_widget::SelectionState<SkillEntry>,

    /// Snapshot of disabled skills before picker opens — restored on ESC.
    /// OWNER: IntentHandler (set on skill picker open, consumed on confirm/cancel).
    pub skill_picker_snapshot: Option<HashSet<String>>,

    /// Preview pane scroll offset for the skill picker.
    /// Reset to 0 when the selection changes.
    pub skill_preview_scroll: usize,

    /// Path to the themes directory (`~/.config/nullslop/themes/`).
    /// Set once during init from `AppPaths`. Used by the theme picker to discover themes.
    /// OWNER: Init code (set once at startup).
    pub themes_dir: std::path::PathBuf,

    /// Path to the system themes directory (`/usr/share/nullslop/themes/`).
    /// Set once during init from `AppPaths`. Used as fallback for theme discovery.
    /// OWNER: Init code (set once at startup).
    pub system_themes_dir: std::path::PathBuf,

    /// Session lifecycle picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (lifecycle picker navigation).
    pub session_lifecycle_picker: nullslop_selection_widget::SelectionState<SessionLifecycleEntry>,

    /// Workflow picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (workflow picker navigation) + WorkflowActor (entry population).
    pub workflow_picker: nullslop_selection_widget::SelectionState<WorkflowPickerEntry>,

    /// Judge picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (judge picker navigation, confirm creates judge session).
    pub judge_picker: nullslop_selection_widget::SelectionState<JudgePickerEntry>,

    /// Compaction model picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (compaction model picker navigation).
    pub compaction_model_picker: nullslop_selection_widget::SelectionState<PickerEntry>,

    /// Arg input popup state — active when `FocusScope::ArgInput` is on the scope stack.
    /// OWNER: IntentHandler (arg input editing, confirmation).
    pub arg_input: ArgInputState,

    /// Rename session input popup state — active when `FocusScope::RenameSessionInput` is on the scope stack.
    /// OWNER: IntentHandler (rename input editing, confirmation).
    pub rename_session_input: RenameSessionInputState,

    /// Sidebar width in columns, synced from preferences.
    /// OWNER: PreferencesStateSyncActor (on PreferencesUpdated).
    pub sidebar_width: u16,

    /// Currently active tab in the main area.
    /// OWNER: IntentHandler (SwitchTab intent).
    pub active_tab: ActiveTab,

    /// Workflow tab UI state — selection, viewport, inspector, cancel prompt.
    /// OWNER: IntentHandler (all workflow UI interactions).
    pub workflow_ui: WorkflowUiState,
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
            tui_signals: TuiSignals::new(),
            preferences: UserPreferences::default(),
            session_picker: nullslop_selection_widget::TreePickerState::new(),
            persona_picker: nullslop_selection_widget::SelectionState::new(),
            scope_stack,
            theme: crate::feat::theme::default_theme(),
            caches: FrontendCaches::default(),
            cancel_stream_prompt: false,
            close_session_prompt: false,
            theme_picker: nullslop_selection_widget::SelectionState::new(),
            theme_preview_original: None,
            tool_picker: nullslop_selection_widget::SelectionState::new(),
            tool_picker_snapshot: None,
            skill_picker: nullslop_selection_widget::SelectionState::new(),
            skill_picker_snapshot: None,
            skill_preview_scroll: 0,
            themes_dir: std::path::PathBuf::new(),
            system_themes_dir: std::path::PathBuf::new(),
            session_lifecycle_picker: nullslop_selection_widget::SelectionState::new(),
            workflow_picker: nullslop_selection_widget::SelectionState::new(),
            judge_picker: nullslop_selection_widget::SelectionState::new(),
            compaction_model_picker: nullslop_selection_widget::SelectionState::new(),
            arg_input: ArgInputState::default(),
            rename_session_input: RenameSessionInputState::default(),
            sidebar_width: 30,
            active_tab: ActiveTab::default(),
            workflow_ui: WorkflowUiState::default(),
        }
    }
}
